import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "../App";

const SESSION = { email: "owner@example.test", display_name: "Alp", role: "owner", via: "password" };


const STATS = {
  sources_total: 14, sources_enabled: 13, sources_verified_live: 12,
  opportunities_open: 1854, opportunities_closed: 0, postings_open: 1905, organizations: 14,
  last_complete_sweep_at: "2026-10-22T09:00:00Z", sources_stale: 0, sources_in_backoff: 0,
  open_topics: 0, next_scheduled_run_at: null,
};

const TASK = {
  id: "task-1",
  application_id: "app-1",
  field_name: "years of relevant experience",
  question_text: "Years of relevant experience",
  field_type: "text",
  required: true,
  sensitive: false,
  draft_answer: null,
  answer: null,
  status: "open",
  created_at: "2026-10-22T09:00:00Z",
  resolved_at: null,
};

const SENSITIVE_TASK = {
  ...TASK,
  id: "task-2",
  field_name: "do you require visa sponsorship",
  question_text: "Do you require visa sponsorship?",
  sensitive: true,
  draft_answer: "No",
};

const APPLICATIONS = [
  {
    id: "app-1", opportunity_id: "opp-1", profile_id: null, status: "needs_review",
    route: "review", route_reason: "1 required question needs an answer from you",
    cv_document_id: null, dossier_body: null, dossier_facts: [], notes: null, follow_up_at: null,
    created_at: "2026-10-22T09:00:00Z", updated_at: "2026-10-22T09:00:00Z",
  },
  {
    id: "app-2", opportunity_id: "opp-2", profile_id: null, status: "submission_unknown",
    route: "auto", route_reason: "every field maps", cv_document_id: null, dossier_body: null,
    dossier_facts: [], notes: null, follow_up_at: null,
    created_at: "2026-10-22T09:00:00Z", updated_at: "2026-10-22T09:00:00Z",
  },
];

const ATTEMPTS = [
  {
    id: "att-1", idempotency_key: "k", state: "submitted", destination: "http://127.0.0.1:8099/apply/single",
    payload_hash: "h", authorization_kind: "policy", receipt: "6a9b5e9b-2016-47d5-88b9-74b77d5278ad",
    diagnostic: null, evidence: {}, started_at: "2026-10-22T09:00:00Z", finished_at: "2026-10-22T09:01:00Z",
  },
];

const POLICIES = [
  {
    id: "pol-1", name: "routine mock applications", enabled: false, source_keys: ["example-greenhouse"],
    require_all_fields_mapped: true, forbid_sensitive_questions: true, forbid_free_text: true,
    max_per_day: 3, max_total: 10, submissions_made: 0, expires_at: "2026-11-22T09:00:00Z",
    revoked_at: null, created_at: "2026-10-22T09:00:00Z",
  },
];

function routes(overrides: Record<string, unknown> = {}) {
  return {
    "/api/auth/session": SESSION, "/api/stats": STATS,
    "/api/review-tasks": [TASK, SENSITIVE_TASK],
    "/api/applications": APPLICATIONS,
    "/api/policies": POLICIES,
    "/api/facts": [],
    "/api/notifications": [],
    ...overrides,
  };
}

function mockFetch(table: Record<string, unknown>, calls?: { method: string; url: string; body?: string }[]) {
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString();
    calls?.push({ method: init?.method ?? "GET", url, body: init?.body as string | undefined });
    const key = Object.keys(table)
      .filter((prefix) => url.startsWith(prefix))
      .sort((a, b) => b.length - a.length)[0];
    if (!key) return new Response("{}", { status: 404, headers: { "content-type": "application/json" } });
    return new Response(JSON.stringify(table[key]), { status: 200, headers: { "content-type": "application/json" } });
  });
}

afterEach(() => vi.unstubAllGlobals());

describe("mobile workspace", () => {
  it("opens on the actions queue and shows what each question needs", async () => {
    vi.stubGlobal("fetch", mockFetch(routes()));
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    await waitFor(() => expect(screen.getByText("Years of relevant experience")).toBeDefined());
    expect(screen.getByText("Do you require visa sponsorship?")).toBeDefined();
    // The sensitive one is called out, because it must come from the person rather than be filled in.
    expect(screen.getByText(/Sensitive, confirm it yourself/)).toBeDefined();
    expect(screen.getByText(/Nothing here was\s+guessed/)).toBeDefined();
  });

  it("answering a question saves the answer and says plainly that nothing was submitted", async () => {
    const calls: { method: string; url: string; body?: string }[] = [];
    vi.stubGlobal("fetch", mockFetch(routes({ "/api/review-tasks/task-1/answer": { ...TASK, status: "answered", answer: "4" } }), calls));
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });

    await waitFor(() => expect(screen.getByText("Years of relevant experience")).toBeDefined());
    const inputs = screen.getAllByRole("textbox");
    fireEvent.change(inputs[0]!, { target: { value: "4" } });
    fireEvent.click(screen.getAllByRole("button", { name: "Save answer" })[0]!);

    // The decisive assertion for this milestone: resolving a review question must never read as having submitted.
    await waitFor(() => expect(screen.getByText(/This application has not been submitted/)).toBeDefined());
    const answerCall = calls.find((c) => c.url.includes("/answer"));
    expect(answerCall?.method).toBe("POST");
    expect(answerCall?.body).toContain("4");
    // Nothing in that flow may call submit.
    expect(calls.some((c) => c.url.includes("/submit"))).toBe(false);
  });

  it("explains every application status rather than showing a bare word", async () => {
    vi.stubGlobal("fetch", mockFetch(routes()));
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    await waitFor(() => expect(screen.getByText("Years of relevant experience")).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Applications" }));

    await waitFor(() => expect(screen.getByText(/Waiting on you/)).toBeDefined());
    // An unconfirmed submission must read as unconfirmed, never as done.
    expect(screen.getByText(/Submitted but unconfirmed. Nobody knows whether it arrived./)).toBeDefined();
  });

  it("shows a submission receipt and a link to the application page", async () => {
    vi.stubGlobal(
      "fetch",
      mockFetch(routes({ "/api/applications/app-1/attempts": ATTEMPTS, "/api/applications/app-1/events": [] })),
    );
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    await waitFor(() => expect(screen.getByText("Years of relevant experience")).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Applications" }));
    await waitFor(() => expect(screen.getAllByRole("button", { name: /History and receipts/ }).length).toBeGreaterThan(0));
    fireEvent.click(screen.getAllByRole("button", { name: /History and receipts/ })[0]!);

    await waitFor(() => expect(screen.getByText(/6a9b5e9b-2016-47d5-88b9-74b77d5278ad/)).toBeDefined());
    expect(screen.getByRole("link", { name: /Open the application page/ })).toBeDefined();
  });

  it("shows a policy as switched off and offers switching on and revoking", async () => {
    const calls: { method: string; url: string; body?: string }[] = [];
    vi.stubGlobal("fetch", mockFetch(routes({ "/api/policies/pol-1/state": { ...POLICIES[0], enabled: true } }), calls));
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    await waitFor(() => expect(screen.getByText("Years of relevant experience")).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Policies" }));

    await waitFor(() => expect(screen.getByText("routine mock applications")).toBeDefined());
    expect(screen.getByText("Switched off")).toBeDefined();
    expect(screen.getByText(/created switched off/)).toBeDefined();

    fireEvent.click(screen.getByRole("button", { name: "Switch on" }));
    await waitFor(() => expect(calls.some((c) => c.url.includes("/policies/pol-1/state"))).toBe(true));
    const body = calls.find((c) => c.url.includes("/state"))?.body ?? "";
    expect(body).toContain('"enabled":true');
    expect(body).toContain('"revoke":false');
  });

  it("refuses to use an unapproved fact and says so on the card", async () => {
    const facts = [
      { id: "f1", key: "full name", value: "A. Example", category: "identity", sensitive: false, approved: true, provenance: "manual", version: 1, created_at: "2026-10-22T09:00:00Z" },
      { id: "f2", key: "years of experience", value: "4", category: "other", sensitive: false, approved: false, provenance: "manual", version: 1, created_at: "2026-10-22T09:00:00Z" },
    ];
    vi.stubGlobal("fetch", mockFetch(routes({ "/api/facts": facts })));
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    await waitFor(() => expect(screen.getByText("Years of relevant experience")).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Facts" }));

    await waitFor(() => expect(screen.getByText("Approved")).toBeDefined());
    expect(screen.getByText("Not approved, will not be used")).toBeDefined();
  });

  it("reads a notification as words rather than as a JSON dump", async () => {
    const notification = {
      id: "note-1",
      application_id: "app-1",
      kind: "submitted",
      dedupe_key: "submitted:app-1",
      payload: { receipt: "2ee2f95b-18c6-4cad-aec7-3bf3f628bff2" },
      created_at: "2026-10-22T09:00:00Z",
      delivered_at: null,
    };
    vi.stubGlobal("fetch", mockFetch(routes({ "/api/notifications": [notification] })));
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    await waitFor(() => expect(screen.getByText("Years of relevant experience")).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Inbox" }));

    await waitFor(() => expect(screen.getByText("2ee2f95b-18c6-4cad-aec7-3bf3f628bff2")).toBeDefined());
    // Braces and quotes on a phone screen are noise; a long receipt must also wrap rather than widen the page.
    expect(screen.queryByText(/[{}"]/)).toBeNull();
  });
});
