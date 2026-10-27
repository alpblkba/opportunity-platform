import type { Page } from "playwright";
import { NavigationBlocked } from "./guard.js";
export type Provider = "greenhouse" | "lever" | "ashby" | "fixture";
export function providerFor(raw: string): Provider | null {
  const u = new URL(raw);
  if (["127.0.0.1", "localhost", "[::1]"].includes(u.hostname)) return "fixture";
  if (u.protocol !== "https:" || u.username || u.password || u.port) return null;
  if (["job-boards.greenhouse.io", "boards.greenhouse.io", "job-boards.eu.greenhouse.io"].includes(u.hostname)) return "greenhouse";
  if (["jobs.lever.co", "jobs.eu.lever.co"].includes(u.hostname)) return "lever";
  if (u.hostname === "jobs.ashbyhq.com") return "ashby";
  return null;
}
export function applicationUrl(raw: string): string {
  const u = new URL(raw), provider = providerFor(raw);
  if (provider === "lever" && !u.pathname.endsWith("/apply")) u.pathname = u.pathname.replace(/\/$/, "") + "/apply";
  if (provider === "ashby" && !u.pathname.endsWith("/application")) u.pathname = u.pathname.replace(/\/$/, "") + "/application";
  return u.href;
}
export function assertSupported(raw: string, submit = false, authorized = false): Provider {
  const provider = providerFor(raw);
  if (!provider) throw new NavigationBlocked(raw, "this application provider requires manual completion");
  if (submit && provider !== "fixture" && (!authorized || process.env.APPLICATION_ENV === "development")) {
    throw new NavigationBlocked(raw, "real submissions need standing authorization and are disabled during development");
  }
  return provider;
}
export async function openForm(page: Page, raw: string): Promise<void> {
  const provider = assertSupported(raw);
  const start = new URL(applicationUrl(raw));
  // Apply the destination boundary to redirects and subrequests too.
  await page.route("**/*", async route => {
    const u = new URL(route.request().url());
    const same = u.origin === start.origin;
    const known = /(^|\.)(greenhouse\.io|lever\.co|ashbyhq\.com|ashbyprd\.com|greenhouse-school\.com|cloudfront\.net|googleapis\.com|gstatic\.com|google\.com|hcaptcha\.com|recaptcha\.net|cloudflare\.com)$/.test(u.hostname);
    if (provider === "fixture" ? same : u.protocol === "https:" && known) await route.continue();
    else await route.abort();
  });
  await page.goto(start.href, { waitUntil: "domcontentloaded", timeout: 30000 });
  assertSupported(page.url());
  await page.locator("form input, form textarea, input[type=file]").first().waitFor({ state: "attached", timeout: 12000 }).catch(() => undefined);
}

export async function receiptOn(page: Page, provider: Provider): Promise<string | null> {
  if (provider === "fixture") return page.locator("#receipt").first().textContent({ timeout: 500 }).catch(() => null);
  const patterns: Record<string, RegExp> = {
    greenhouse: /thank you for applying|your application has been (received|submitted)|application submitted successfully/i,
    lever: /application (has been )?(submitted|received)|thanks for applying|thank you for applying/i,
    ashby: /application (has been )?(submitted|received)|thank you for applying|thanks for applying/i,
  };
  // Require a new visible confirmation after submission, never script source or hidden boilerplate.
  const text = await page.locator("body").innerText();
  const match = text.match(patterns[provider]!);
  return match ? `${provider}: ${match[0]} · ${page.url()}` : null;
}
