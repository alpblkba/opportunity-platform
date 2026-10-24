/**
 * Playwright worker.
 *
 * Three operations: inspect a form into a plan, fill it from values the Rust side already decided, and submit it when an authorization says so. The worker never decides what a field should contain and never decides whether to submit; it reports what it sees and does what it is told, which keeps every judgement in one place on the Rust side where it is tested.
 *
 * Screenshots and traces go under the private tree and are never logged.
 */
import { chromium, type Browser, type Page } from "playwright";
import { createHash } from "node:crypto";
import { writeFileSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";

import { assertNavigable } from "./guard.js";

export type FieldKind =
  | "text" | "email" | "phone" | "url" | "select" | "checkbox" | "radio" | "textarea" | "file" | "unsupported";

export interface FormField {
  name: string;
  label: string;
  kind: FieldKind;
  required: boolean;
  options: string[];
  max_length: number | null;
}

export interface FormPlan {
  url: string;
  fields: FormField[];
  steps: number;
  has_captcha: boolean;
  requires_login: boolean;
  submit_selector: string | null;
}

/** Where private artefacts go. Never inside the repository tree that Git can see. Read per call so a long-running worker picks up a reconfigured location without a restart. */
function artefactRoot(): string {
  return process.env.BROWSER_WORKER_ARTEFACTS ?? "private/browser";
}

function kindOf(type: string | null, tag: string): FieldKind {
  if (tag === "textarea") return "textarea";
  if (tag === "select") return "select";
  switch (type) {
    case "email": return "email";
    case "tel": return "phone";
    case "url": return "url";
    case "checkbox": return "checkbox";
    case "radio": return "radio";
    case "file": return "file";
    case "text": case "search": case null: case undefined: return "text";
    // A control this worker has no safe way to operate is reported as such rather than guessed at.
    default: return "unsupported";
  }
}

/**
 * Read every control currently on the page.
 *
 * Multi-step forms hide later steps, so the caller advances the form and inspects again; a field that only appears after step one is therefore seen, which is the whole point of the dynamic-required case.
 */
async function readFields(page: Page): Promise<FormField[]> {
  return page.evaluate(() => {
    const visible = (el: Element): boolean => {
      const node = el as HTMLElement;
      if (node.hidden) return false;
      let parent: HTMLElement | null = node;
      while (parent) {
        if (parent.hidden) return false;
        parent = parent.parentElement;
      }
      return true;
    };
    const labelFor = (el: Element): string => {
      const id = el.getAttribute("id");
      if (id) {
        const explicit = document.querySelector(`label[for="${id}"]`);
        if (explicit?.textContent) return explicit.textContent.trim();
      }
      const wrapping = el.closest("label");
      if (wrapping?.textContent) return wrapping.textContent.trim();
      return el.getAttribute("name") ?? el.getAttribute("id") ?? "";
    };
    const out: Array<Record<string, unknown>> = [];
    for (const el of document.querySelectorAll("input, select, textarea")) {
      if (!visible(el)) continue;
      const type = el.getAttribute("type");
      if (type === "hidden" || type === "submit" || type === "button") continue;
      const maxLength = el.getAttribute("maxlength");
      out.push({
        name: el.getAttribute("name") ?? el.getAttribute("id") ?? "",
        label: labelFor(el),
        tag: el.tagName.toLowerCase(),
        type,
        required: el.hasAttribute("required"),
        options: el.tagName.toLowerCase() === "select"
          ? [...(el as HTMLSelectElement).options].map((o) => o.value).filter(Boolean)
          : [],
        max_length: maxLength ? Number(maxLength) : null,
      });
    }
    return out;
  }) as unknown as Promise<FormField[]>;
}

async function normalizeFields(page: Page): Promise<FormField[]> {
  const raw = (await readFields(page)) as unknown as Array<{
    name: string; label: string; tag: string; type: string | null; required: boolean; options: string[]; max_length: number | null;
  }>;
  return raw
    .filter((f) => f.name)
    .map((f) => ({
      name: f.name,
      label: f.label || f.name,
      kind: kindOf(f.type, f.tag),
      required: f.required,
      options: f.options,
      max_length: f.max_length,
    }));
}

/** Buttons that advance a multi-step form. Matched by visible text so the worker is not tied to one site's markup. */
const NEXT_LABELS = ["next", "continue", "weiter"];

async function advance(page: Page): Promise<boolean> {
  for (const label of NEXT_LABELS) {
    const button = page.locator(`button:visible, input[type=button]:visible`).filter({ hasText: new RegExp(label, "i") });
    if ((await button.count()) > 0) {
      await button.first().click();
      await page.waitForTimeout(100);
      return true;
    }
  }
  return false;
}

/** Inspect a form, walking every step so dynamically revealed fields are included. */
export async function inspect(url: string, maxSteps = 6): Promise<FormPlan> {
  assertNavigable(url);
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage();
    await page.goto(url, { waitUntil: "domcontentloaded" });

    const byName = new Map<string, FormField>();
    let steps = 0;
    for (let step = 0; step < maxSteps; step += 1) {
      steps = step + 1;
      for (const field of await normalizeFields(page)) {
        if (!byName.has(field.name)) byName.set(field.name, field);
      }
      if (!(await advance(page))) break;
    }

    const body = (await page.content()).toLowerCase();
    return {
      url,
      fields: [...byName.values()],
      steps,
      // A challenge is reported, never attempted. Its presence is what routes the application to a human.
      has_captcha: /captcha|recaptcha|hcaptcha|turnstile/.test(body),
      requires_login: /type="password"|sign in to apply|log in to apply/.test(body),
      submit_selector: (await page.locator("button[type=submit]:visible").count()) > 0 ? "button[type=submit]" : null,
    };
  } finally {
    await browser.close();
  }
}

export interface FillValue {
  field: string;
  value: string;
}

export interface FillRequest {
  url: string;
  values: FillValue[];
  /** Absolute path to an approved document to attach, when the form takes one. */
  documentPath?: string;
  /** Identifies the artefact directory. Screenshots land under the private tree keyed by this. */
  reference: string;
}

export interface FillResult {
  filled: string[];
  skipped: string[];
  screenshot: string;
  payloadHash: string;
}

async function setValue(page: Page, field: string, value: string): Promise<boolean> {
  const escaped = field.replace(/"/g, '\\"');
  const locator = page.locator(`[name="${escaped}"]:visible`).first();
  if ((await locator.count()) === 0) return false;
  const tag = await locator.evaluate((el) => el.tagName.toLowerCase());
  const type = await locator.getAttribute("type");
  if (tag === "select") {
    await locator.selectOption(value);
  } else if (type === "checkbox") {
    // Only an explicit affirmative ticks a box. Anything else leaves it alone rather than agreeing on the owner's behalf.
    if (["yes", "true", "on", "1", "agree"].includes(value.trim().toLowerCase())) await locator.check();
  } else {
    await locator.fill(value);
  }
  return true;
}

/** Fill a form and stop. This never clicks submit, whatever the form offers. */
export async function fill(request: FillRequest): Promise<FillResult> {
  assertNavigable(request.url);
  const directory = join(artefactRoot(), request.reference);
  await mkdir(directory, { recursive: true });

  const browser: Browser = await chromium.launch();
  try {
    const page = await browser.newPage();
    await page.goto(request.url, { waitUntil: "domcontentloaded" });

    const filled: string[] = [];
    const skipped: string[] = [];
    for (let step = 0; step < 6; step += 1) {
      for (const { field, value } of request.values) {
        if (filled.includes(field)) continue;
        if (await setValue(page, field, value)) filled.push(field);
      }
      if (request.documentPath) {
        const file = page.locator("input[type=file]:visible").first();
        if ((await file.count()) > 0 && !filled.includes("__document")) {
          await file.setInputFiles(request.documentPath);
          filled.push("__document");
        }
      }
      if (!(await advance(page))) break;
    }
    for (const { field } of request.values) if (!filled.includes(field)) skipped.push(field);

    const screenshot = join(directory, "before-submit.png");
    await page.screenshot({ path: screenshot, fullPage: true });
    return { filled, skipped, screenshot, payloadHash: hashValues(request.values) };
  } finally {
    await browser.close();
  }
}

export interface SubmitRequest extends FillRequest {
  /** The payload hash the authorization was issued against. A mismatch stops the submission. */
  authorizedPayloadHash: string;
  submitTimeoutMs?: number;
  /**
   * A file to create immediately before the submit control is clicked.
   *
   * It is the only thing that survives this process being killed. If the machine runs out of memory mid-submission, the caller finds this file and knows a submission may have left; without it, the caller knows one did not. Written synchronously for that reason: a buffered write would be exactly the thing lost.
   */
  progressPath?: string;
}

export type SubmitOutcome =
  | { state: "submitted"; receipt: string; screenshot: string }
  | { state: "unknown"; diagnostic: string; screenshot: string }
  | { state: "failed"; diagnostic: string };

/**
 * Fill and submit.
 *
 * Three outcomes and no others. A receipt means it landed. No answer after the click means nobody knows, which is reported as unknown so a human reconciles it; the worker never retries, because a retry after an uncertain submit is how someone applies twice.
 */
export async function submit(request: SubmitRequest): Promise<SubmitOutcome> {
  assertNavigable(request.url);
  const actual = hashValues(request.values);
  if (actual !== request.authorizedPayloadHash) {
    return { state: "failed", diagnostic: "the answers changed after this submission was authorized" };
  }

  const directory = join(artefactRoot(), request.reference);
  await mkdir(directory, { recursive: true });
  const browser = await chromium.launch();
  let clicked = false;
  try {
    const page = await browser.newPage();
    await page.goto(request.url, { waitUntil: "domcontentloaded" });
    for (let step = 0; step < 6; step += 1) {
      for (const { field, value } of request.values) await setValue(page, field, value);
      if (request.documentPath) {
        const file = page.locator("input[type=file]:visible").first();
        if ((await file.count()) > 0) await file.setInputFiles(request.documentPath);
      }
      if (!(await advance(page))) break;
    }

    const submitButton = page.locator("button[type=submit]:visible").first();
    if ((await submitButton.count()) === 0) {
      return { state: "failed", diagnostic: "no submit control was found on the form" };
    }

    const timeout = request.submitTimeoutMs ?? 8000;
    // From here on, nobody can prove nothing was sent.
    if (request.progressPath) writeFileSync(request.progressPath, new Date().toISOString());
    clicked = true;
    try {
      // Waiting for the navigation the submission causes, not for a load state: the form page has already loaded, so a load state resolves at once and would hide a server that never answers.
      await Promise.all([
        page.waitForURL((candidate) => candidate.pathname.startsWith("/submit"), { timeout }),
        submitButton.click(),
      ]);
    } catch {
      const screenshot = join(directory, "after-submit-unknown.png");
      await page.screenshot({ path: screenshot, fullPage: true }).catch(() => undefined);
      // The distinction that matters most in this whole worker. If the browser's own validation refused the form, nothing left the machine and trying again is safe, so this is a failure. If the form was valid and the site simply never answered, nobody knows whether it landed, and that must block retries.
      const invalid = await withTimeout(
        page.evaluate(() => {
          const form = document.querySelector("form");
          if (!form || form.checkValidity()) return null;
          return [...form.elements]
            .filter((el) => typeof (el as HTMLObjectElement).checkValidity === "function")
            .filter((el) => !(el as HTMLObjectElement).checkValidity())
            .map((el) => el.getAttribute("name") ?? el.getAttribute("id") ?? "unnamed");
        }),
        2000,
      ).catch(() => null);
      if (invalid && invalid.length > 0) {
        return {
          state: "failed",
          diagnostic: `the form refused to submit because these required fields were not accepted: ${invalid.join(", ")}. Nothing was sent.`,
        };
      }
      return {
        state: "unknown",
        diagnostic: "the form was submitted but the site did not answer, so whether it was recorded is unknown",
        screenshot,
      };
    }

    const screenshot = join(directory, "after-submit.png");
    await page.screenshot({ path: screenshot, fullPage: true }).catch(() => undefined);
    const receipt = await page
      .locator("#receipt")
      .first()
      .textContent({ timeout })
      .catch(() => null);
    if (!receipt) {
      // The page answered but produced no confirmation. That is not success.
      return {
        state: "unknown",
        diagnostic: "the site answered but published no confirmation reference",
        screenshot,
      };
    }
    return { state: "submitted", receipt: receipt.trim(), screenshot };
  } catch (error) {
    // Anything that goes wrong after the click is an uncertain outcome, not a failure. A renderer killed by the kernel reaches here, and "the browser crashed" says nothing about whether the form had already gone.
    if (clicked) {
      return {
        state: "unknown",
        diagnostic: `the browser failed after the form was submitted, so whether it was recorded is unknown: ${String(error)}`,
        screenshot: join(directory, "after-submit-unknown.png"),
      };
    }
    throw error;
  } finally {
    await browser.close();
  }
}

/** Resolve a promise or give up, so a page stuck mid-navigation cannot hold the worker for the default thirty seconds. */
function withTimeout<T>(promise: Promise<T>, ms: number): Promise<T> {
  return Promise.race([
    promise,
    new Promise<T>((_, reject) => setTimeout(() => reject(new Error("timed out")), ms)),
  ]);
}

/**
 * Fingerprint of exactly what will be submitted.
 *
 * The Rust side computes this identically in `payload_hash`. Both must agree, because this worker refuses to submit when the payload it holds does not hash to the value the authorization was issued against. Fields sorted by name, one `field=value` per line, joined by newlines, SHA-256, lowercase hex.
 */
export function hashValues(values: FillValue[]): string {
  const canonical = [...values]
    .sort((a, b) => (a.field < b.field ? -1 : a.field > b.field ? 1 : 0))
    .map((v) => `${v.field}=${v.value}`)
    .join("\n");
  return createHash("sha256").update(canonical, "utf8").digest("hex");
}
