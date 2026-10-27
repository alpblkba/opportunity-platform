/**
 * Playwright worker.
 *
 * Three operations: inspect a form into a plan, fill it from values the Rust side already decided, and submit it when an authorization says so. The worker never decides what a field should contain and never decides whether to submit; it reports what it sees and does what it is told, which keeps every judgement in one place on the Rust side where it is tested.
 *
 * Screenshots and traces go under the private tree and are never logged.
 */
import { chromium, type Browser, type Page } from "playwright";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { join } from "node:path";

import { assertSupported, openForm, receiptOn } from "./providers.js";

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
      return getComputedStyle(node).display !== "none" && getComputedStyle(node).visibility !== "hidden";
    };
    const labelFor = (el: Element): string => {
      const groupLabel = el.closest("[data-field-path]")?.querySelector("label");
      if (groupLabel?.textContent) return groupLabel.textContent.trim();
      const id = el.getAttribute("id");
      if (id) {
        const explicit = document.querySelector(`label[for="${CSS.escape(id)}"]`);
        if (explicit?.textContent) return explicit.textContent.trim();
      }
      if (el.getAttribute("type") === "radio") {
        const group = el.closest("fieldset, .application-question, [role=radiogroup]");
        const heading = group?.querySelector("legend, .application-label, [data-qa=question-label]");
        if (heading?.textContent) return heading.textContent.trim();
      }
      const wrapping = el.closest("label");
      if (wrapping?.textContent) return wrapping.textContent.trim();
      return el.getAttribute("aria-label") ?? el.getAttribute("placeholder") ?? el.getAttribute("name") ?? el.getAttribute("id") ?? "";
    };
    const out: Array<Record<string, unknown>> = [];
    for (const el of document.querySelectorAll("input, select, textarea")) {
      const group = el.closest("[data-field-path]");
      const yesno = !!group?.querySelector("button[data-option]");
      if (!visible(el) && el.getAttribute("type") !== "file" && !yesno) continue;
      const type = el.getAttribute("type");
      if (type === "hidden" || type === "submit" || type === "button") continue;
      const maxLength = el.getAttribute("maxlength");
      out.push({
        name: el.getAttribute("name") || el.getAttribute("id") || group?.getAttribute("data-field-path") || "",
        label: labelFor(el),
        tag: el.tagName.toLowerCase(),
        type: yesno ? "radio" : el.getAttribute("role") === "combobox" && el.tagName.toLowerCase() !== "select" ? "custom-combobox" : type,
        required: el.hasAttribute("required") || el.getAttribute("aria-required") === "true" || /[*✱]/.test(labelFor(el)) || !!group?.querySelector('label[class*="_required_"]'),
        options: yesno ? ["Yes", "No"] : el.tagName.toLowerCase() === "select"
          ? [...(el as HTMLSelectElement).options].map((o) => o.value).filter(Boolean)
          : type === "radio" ? [...document.querySelectorAll("input[type=radio]")].filter(r => r.getAttribute("name") === el.getAttribute("name")).map(r => r.getAttribute("value") || "") : type === "checkbox" ? ["Yes", "No"] : [],
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
const SUBMIT_SELECTOR = 'button[type=submit]:visible, input[type=submit]:visible, button:has-text("Submit Application"):visible, button:has-text("Bewerbung absenden"):visible';

async function advance(page: Page): Promise<boolean> {
  for (const label of NEXT_LABELS) {
    const button = page.locator(`button[type=button]:visible, input[type=button]:visible`).filter({ hasText: new RegExp(`^${label}$`, "i") });
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
  assertSupported(url);
  const browser = await chromium.launch();
  try {
    const page = await browser.newPage();
    await openForm(page, url);

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
      has_captcha: (await page.locator('iframe[src*="captcha"], iframe[src*="turnstile"], [data-sitekey]').count()) > 0 || /verify you are human|complete the captcha/.test(body),
      requires_login: /type="password"|sign in to apply|log in to apply/.test(body),
      submit_selector: (await page.locator(SUBMIT_SELECTOR).count()) > 0 ? SUBMIT_SELECTOR : null,
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
  standingAuthorized?: boolean;
  documentHash?: string;
}

export interface FillResult {
  filled: string[];
  skipped: string[];
  screenshot: string;
  payloadHash: string;
}

async function setValue(page: Page, field: string, value: string): Promise<boolean> {
  const escaped = field.replace(/"/g, '\\"');
  const customOptions = page.locator(`[data-field-path="${escaped}"] button[data-option]`);
  if (await customOptions.count()) {
    for (const option of await customOptions.all()) {
      if ((await option.getAttribute("data-option"))?.toLowerCase() === value.toLowerCase()) { await option.click(); return true; }
    }
    return false;
  }
  const locator = page.locator(`[name="${escaped}"]:visible, [id="${escaped}"]:visible`).first();
  if ((await locator.count()) === 0) return false;
  const tag = await locator.evaluate((el) => el.tagName.toLowerCase());
  const type = await locator.getAttribute("type");
  if (tag === "select") {
    await locator.selectOption(value);
  } else if (type === "radio") {
    const radio = page.locator(`input[name="${escaped}"][type=radio]`).filter({ visible: true });
    for (const el of await radio.all()) if (await el.getAttribute("value") === value) { await el.check(); return true; }
    return false;
  } else if (type === "checkbox") {
    // Only an explicit affirmative ticks a box. Anything else leaves it alone rather than agreeing on the owner's behalf.
    await locator.setChecked(["yes", "true", "on", "1", "agree"].includes(value.trim().toLowerCase()));
  } else {
    await locator.fill(value);
  }
  return true;
}

/** Fill a form and stop. This never clicks submit, whatever the form offers. */
export async function fill(request: FillRequest): Promise<FillResult> {
  assertSupported(request.url);
  const directory = join(artefactRoot(), request.reference);
  await mkdir(directory, { recursive: true });

  const browser: Browser = await chromium.launch();
  try {
    const page = await browser.newPage();
    await openForm(page, request.url);

    const filled: string[] = [];
    const skipped: string[] = [];
    for (let step = 0; step < 6; step += 1) {
      for (const { field, value } of request.values) {
        if (filled.includes(field)) continue;
        if (await setValue(page, field, value)) filled.push(field);
      }
      if (request.documentPath) {
        const file = page.locator('input[type=file][name*=resume i],input[type=file][name*=cv i],input[type=file][id*=resume i],input[type=file][id*=cv i]').first();
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
  const provider = assertSupported(request.url, true, request.standingAuthorized === true);
  let actual = hashValues(request.values);
  if (request.documentHash) {
    if (!request.documentPath || createHash("sha256").update(readFileSync(request.documentPath)).digest("hex") !== request.documentHash) {
      return {state:"failed", diagnostic:"The CV changed after authorization."};
    }
    actual = createHash("sha256").update(`${actual}\ncv=${request.documentHash}`).digest("hex");
  }
  if (actual !== request.authorizedPayloadHash) {
    return { state: "failed", diagnostic: "the answers changed after this submission was authorized" };
  }

  const directory = join(artefactRoot(), request.reference);
  await mkdir(directory, { recursive: true });
  const browser = await chromium.launch();
  let clicked = false;
  try {
    const page = await browser.newPage();
    await openForm(page, request.url);
    for (let step = 0; step < 6; step += 1) {
      for (const { field, value } of request.values) await setValue(page, field, value);
      if (request.documentPath) {
        const file = page.locator('input[type=file][name*=resume i],input[type=file][name*=cv i],input[type=file][id*=resume i],input[type=file][id*=cv i]').first();
        if ((await file.count()) > 0) await file.setInputFiles(request.documentPath);
      }
      if (!(await advance(page))) break;
    }

    const submitButton = page.locator(SUBMIT_SELECTOR).first();
    if ((await submitButton.count()) === 0) {
      return { state: "failed", diagnostic: "no submit control was found on the form" };
    }

    const body = (await page.locator("body").innerText()).toLowerCase();
    const challenge = await page.locator('iframe[src*="captcha"], iframe[src*="turnstile"], [data-sitekey]').count();
    if (challenge || /verify you are human|complete the captcha/.test(body)) return {state:"failed",diagnostic:"The employer requires a verification challenge. Nothing was sent."};
    if (await receiptOn(page, provider)) return {state:"failed",diagnostic:"Confirmation was already present before submission; refusing an ambiguous form."};
    const missing = await page.locator("input:invalid, select:invalid, textarea:invalid").count();
    if (missing) return {state:"failed",diagnostic:"Required fields were not accepted. Nothing was sent."};
    const timeout = request.submitTimeoutMs ?? 12000;
    if (request.progressPath) writeFileSync(request.progressPath, new Date().toISOString());
    clicked = true;
    await submitButton.click({timeout});
    const deadline = Date.now() + timeout;
    let receipt: string | null = null;
    while (Date.now() < deadline && !receipt) {
      receipt = await receiptOn(page, provider).catch(() => null);
      if (!receipt) await page.waitForTimeout(200);
    }
    const screenshot = join(directory, receipt ? "after-submit.png" : "after-submit-unknown.png");
    await page.screenshot({ path: screenshot, fullPage: true }).catch(() => undefined);
    if (!receipt) return {state:"unknown",diagnostic:"No employer confirmation was observed. Check with the employer before retrying.",screenshot};
    return {state:"submitted",receipt:receipt.trim(),screenshot};
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
