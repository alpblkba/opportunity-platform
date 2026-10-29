import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "../App";

const job = (id: string, title: string, extra = {}) => ({ id, canonical_title: title, organization: "Example Silicon", provider: "lever", source_key: "example-lever",
  apply_url: `https://jobs.lever.co/example/${id}`, first_seen_at: "2026-10-28T09:00:00Z", last_seen_at: "2026-10-29T08:00:00Z", deadline_at: null,
  score: 80, technical: 70, profile_match: 40, career_stage: "Internship", career_priority: 100, eligibility: "eligible", explanation: "Your CV: C, RTOS", gaps: [], ...extra });
const prefs = { new_match_digest: true, min_priority: 60, digest_hour: 8, time_zone: "Europe/Berlin", deadline_reminders: true, reminder_days: [7, 3, 1] };
const home = { cv: { id: "cv1", label: "stored-cv.pdf", created_at: "2026-10-25T08:00:00Z" }, cv_ready: true,
  profiles: [{ id: "firmware", name: "Firmware and platform systems" }], selected_profile: "firmware",
  jobs: [job("j1", "Firmware Intern"), job("j2", "Embedded Engineer", { first_seen_at: "2026-10-20T09:00:00Z", deadline_at: "2026-11-15T12:00:00Z", career_stage: "Junior" })],
  matches: 2, total_ranked: 2, discovered: 10, applications: [],
  source_health: { enabled: 3, failing: 0, never_succeeded: 0, stale: 0, last_success_at: null },
  application_counts: { confirmed: 0, reported: 0, uncertain: 0, failed: 0, needs_review: 0, queued: 0, manual: 0 },
  questions: [], policies: [], mail: [], email_configured: false, last_updated: null,
  filter_options: { stages: ["Internship", "Junior"], countries: ["DE"], sources: ["example-lever"] },
  marks: [], exclusions: [], previous_visit_at: "2026-10-25T00:00:00Z", notification_preferences: prefs };

type Handler = (path: string, init?: RequestInit) => unknown;
function serve(overrides: Record<string, Handler> = {}) {
  const fetch = vi.fn(async (input: string, init?: RequestInit) => {
    const path = String(input);
    for (const [prefix, handler] of Object.entries(overrides)) if (path.startsWith(prefix)) {
      const result = handler(path, init);
      return result instanceof Response ? result : Response.json(result ?? {});
    }
    if (path === "/api/auth/session") return Response.json({ email: "owner@example.test", display_name: "Owner", via: "password" });
    if (path.startsWith("/api/home?") || path === "/api/home") return Response.json(home);
    return Response.json({});
  });
  vi.stubGlobal("fetch", fetch);
  return fetch;
}
const calls = (fetch: ReturnType<typeof serve>, prefix: string) => fetch.mock.calls.filter(c => String(c[0]).startsWith(prefix));
beforeEach(() => window.history.replaceState({}, "", "/workspace"));
afterEach(() => { cleanup(); vi.unstubAllGlobals(); vi.useRealTimers(); });

it("marks matches first seen since the previous visit as new, and records this visit once", async () => {
  const fetch = serve();
  render(<App />);
  const intern = (await screen.findByText("Firmware Intern")).closest("article")!;
  expect(within(intern).getByText("New")).toBeDefined();
  const older = screen.getByText("Embedded Engineer").closest("article")!;
  expect(within(older).queryByText("New")).toBeNull();
  expect(within(older).getByText(/Deadline/)).toBeDefined();
  await waitFor(() => expect(calls(fetch, "/api/home/visited")).toHaveLength(1));
});

it("sends filters to the server and says how many of all matches are shown", async () => {
  const fetch = serve({ "/api/home?": () => ({ ...home, jobs: [home.jobs[0]], matches: 1 }) });
  render(<App />);
  await screen.findByText("Firmware Intern");
  fireEvent.change(screen.getByLabelText("Career stage"), { target: { value: "Internship" } });
  fireEvent.click(screen.getByLabelText("Saved only"));
  await waitFor(() => expect(calls(fetch, "/api/home?").some(c => String(c[0]).includes("stage=Internship") && String(c[0]).includes("saved=true"))).toBe(true));
  expect(await screen.findByText("Showing 1 of 2")).toBeDefined();
});

it("saves, dismisses and hides an employer through the server, and undoes a dismissal", async () => {
  let rules: unknown[] = [];
  const fetch = serve({
    "/api/home/mark/": () => ({ ok: true }),
    "/api/home/dismiss/": () => { rules = [{ id: "r1", kind: "hidden", value: "", opportunity_id: "j1", title: "Firmware Intern" }]; return rules[0]; },
    "/api/home/block-employer": () => ({ id: "r2", kind: "company", value: "Example Silicon" }),
    "/api/rules/": () => { rules = []; return new Response(null, { status: 204 }); },
    "/api/home": () => ({ ...home, exclusions: rules }),
  });
  render(<App />);
  const intern = (await screen.findByText("Firmware Intern")).closest("article")!;
  fireEvent.click(within(intern).getByRole("button", { name: "Save" }));
  await screen.findByText(/Saved\. Deadline reminders/);
  expect(JSON.parse(String(calls(fetch, "/api/home/mark/j1")[0]?.[1]?.body))).toEqual({ mark: "saved", on: true });
  fireEvent.click(within(intern).getByRole("button", { name: "Dismiss" }));
  await screen.findByText(/Dismissed\. Undo it/);
  fireEvent.click(within(intern).getByText("More"));
  fireEvent.click(within(intern).getByRole("button", { name: "Hide all from Example Silicon" }));
  await screen.findByText(/hidden from every profile/);
  expect(JSON.parse(String(calls(fetch, "/api/home/block-employer")[0]?.[1]?.body))).toEqual({ employer: "Example Silicon" });
  fireEvent.click(await screen.findByText(/Dismissed and hidden/, { selector: "summary" }));
  fireEvent.click(screen.getByRole("button", { name: "Undo" }));
  await screen.findByText(/Restored\./);
  expect(calls(fetch, "/api/rules/r1")[0]?.[1]?.method).toBe("DELETE");
});

it("saves alert preferences and shows the server's refusal instead of a success", async () => {
  let reject = true;
  const fetch = serve({ "/api/home/notifications": (_p, init) => {
    if (reject) { reject = false; return Response.json({ detail: "Use a time zone name such as Europe/Berlin." }, { status: 400 }); }
    return JSON.parse(String(init?.body));
  } });
  render(<App />);
  fireEvent.click(await screen.findByText("Email alerts"));
  fireEvent.change(screen.getByLabelText("Time zone"), { target: { value: "Mars/Olympus" } });
  fireEvent.click(screen.getByRole("button", { name: "Save alerts" }));
  expect(await screen.findByText("Use a time zone name such as Europe/Berlin.")).toBeDefined();
  fireEvent.change(screen.getByLabelText("Time zone"), { target: { value: "Europe/Berlin" } });
  fireEvent.change(screen.getByLabelText("Days before the deadline (comma separated)"), { target: { value: "14, 2" } });
  fireEvent.click(screen.getByRole("button", { name: "Save alerts" }));
  expect(await screen.findByText("Alert preferences saved.")).toBeDefined();
  expect(JSON.parse(String(calls(fetch, "/api/home/notifications").at(-1)?.[1]?.body)).reminder_days).toEqual([14, 2]);
});

it("adds a fact unapproved and approves it only when asked", async () => {
  let facts = [{ id: "f1", key: "email", value: "owner@example.test", category: "contact", sensitive: false, approved: true, provenance: "CV", version: 1 }];
  const fetch = serve({
    "/api/facts/": (path, init) => { const id = path.split("/")[3]; facts = facts.map(f => f.id === id ? { ...f, approved: JSON.parse(String(init?.body)).approved } : f); return facts.find(f => f.id === id); },
    "/api/facts": (_p, init) => { if (init?.method === "POST") { const body = JSON.parse(String(init.body)); facts = [...facts, { id: "f2", ...body, sensitive: false, approved: false, version: 1 }]; return facts[1]; } return facts; },
  });
  render(<App />);
  fireEvent.click(await screen.findByText(/About you/));
  await screen.findByText("owner@example.test");
  fireEvent.change(screen.getByLabelText(/^Fact/), { target: { value: "LinkedIn" } });
  fireEvent.change(screen.getByLabelText("Value"), { target: { value: "https://www.linkedin.com/in/example" } });
  fireEvent.change(screen.getByLabelText("Kind"), { target: { value: "contact" } });
  fireEvent.click(screen.getByRole("button", { name: "Add or correct fact" }));
  expect(await screen.findByText(/Approve it before it is used/)).toBeDefined();
  expect(JSON.parse(String(calls(fetch, "/api/facts").find(c => c[1]?.method === "POST")?.[1]?.body))).toMatchObject({ key: "LinkedIn", category: "contact" });
  const linkedin = (await screen.findByText("LinkedIn")).closest("li")!;
  expect(within(linkedin).getByText(/not approved/)).toBeDefined();
  fireEvent.click(within(linkedin).getByRole("button", { name: "Approve" }));
  expect(await screen.findByText("LinkedIn approved for applications.")).toBeDefined();
});

it("tracks a job from elsewhere without sending anything, and shows the manual checklist with the CV used", async () => {
  let tracked = 0;
  const manual = { id: "a1", opportunity_id: "o1", status: "unsupported", notes: null, reason: "Apply on www.linkedin.com, then record it here.", title: "Firmware Engineer", employer: "Example Silicon", url: "https://www.linkedin.com/jobs/view/4012345678/", cv_label: "older-cv.pdf" };
  const fetch = serve({
    "/api/home/track-external": () => ({ application_id: "a1", duplicate: tracked++ > 0 }),
    "/api/home": () => ({ ...home, applications: tracked ? [manual] : [] }),
  });
  render(<App />);
  fireEvent.click(await screen.findByText("Track a job from LinkedIn or elsewhere"));
  const fill = () => {
    fireEvent.change(screen.getByLabelText("Job title"), { target: { value: "Firmware Engineer" } });
    fireEvent.change(screen.getByLabelText("Employer"), { target: { value: "Example Silicon" } });
    fireEvent.change(screen.getByLabelText("Link to the job"), { target: { value: "https://www.linkedin.com/jobs/view/4012345678/" } });
    fireEvent.click(screen.getByRole("button", { name: "Track it" }));
  };
  fill();
  expect(await screen.findByText(/Tracked\. Follow its checklist/)).toBeDefined();
  expect(JSON.parse(String(calls(fetch, "/api/home/track-external")[0]?.[1]?.body))).toEqual({ title: "Firmware Engineer", employer: "Example Silicon", url: "https://www.linkedin.com/jobs/view/4012345678/", deadline_at: null });
  const checklist = await screen.findByRole("list", { name: "Checklist for Firmware Engineer" });
  expect(within(checklist).getByRole("link", { name: /Open the job/ }).getAttribute("href")).toBe(manual.url);
  expect(within(checklist).getByText(/“older-cv\.pdf”; a newer CV, “stored-cv\.pdf”, is stored/)).toBeDefined();
  fill();
  expect(await screen.findByText("Already tracked; nothing was duplicated.")).toBeDefined();
  expect(calls(fetch, "/api/applications/")).toHaveLength(0);
});

it("hides postings by a word in the title or description through the rules API", async () => {
  const fetch = serve({ "/api/rules": (_p, init) => ({ id: "r3", ...JSON.parse(String(init?.body)) }) });
  render(<App />);
  fireEvent.click(await screen.findByText(/Dismissed and hidden/, { selector: "summary" }));
  fireEvent.change(screen.getByLabelText("Hide postings containing a word"), { target: { value: " sales " } });
  fireEvent.change(screen.getByLabelText("Look for it in"), { target: { value: "description_term" } });
  fireEvent.click(screen.getByRole("button", { name: "Hide them" }));
  expect(await screen.findByText("Postings with “sales” in the description are hidden from every profile.")).toBeDefined();
  expect(JSON.parse(String(calls(fetch, "/api/rules")[0]?.[1]?.body))).toEqual({ kind: "description_term", value: "sales", note: "Added in the workspace" });
});
