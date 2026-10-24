import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "../App";

const SESSION = { email: "owner@example.test", display_name: "Alp", role: "owner", via: "password" };


const STATS = {
  sources_total: 5,
  sources_enabled: 4,
  sources_verified_live: 4,
  opportunities_open: 2,
  opportunities_closed: 0,
  postings_open: 2,
  organizations: 2,
  last_complete_sweep_at: "2026-10-22T09:00:00Z",
  sources_stale: 1,
  sources_in_backoff: 0,
  open_topics: 1,
  next_scheduled_run_at: "2026-10-22T15:00:00Z",
};

const OPPORTUNITIES = {
  total: 2,
  page: 1,
  per_page: 25,
  items: [
    {
      id: "11111111-1111-1111-1111-111111111111",
      canonical_title: "Firmware Engineer, Platform",
      organization: "Example Silicon",
      kind: "employment",
      advertised_state: "advertised",
      country_code: "DE",
      city: "Munich",
      remote_raw: null,
      deadline_at: null,
      supervisor: null,
      contact: null,
      first_seen_at: "2026-10-20T09:00:00Z",
      last_seen_at: "2026-10-22T09:00:00Z",
      closed_at: null,
      title_only: false,
      posting_count: 2,
      apply_url: "https://example.org/jobs/1",
      source_names: ["Example Silicon"],
      posted_at: "2026-09-01T09:00:00Z",
    },
    {
      id: "22222222-2222-2222-2222-222222222222",
      canonical_title: "Masterarbeit: Speicherhierarchien",
      organization: "Example University",
      kind: "master_thesis",
      advertised_state: "open_topic",
      country_code: "DE",
      city: "Karlsruhe",
      remote_raw: null,
      deadline_at: "2026-12-31T23:59:59Z",
      supervisor: "Prof. Dr. Anna Beispiel",
      contact: null,
      first_seen_at: "2026-10-21T09:00:00Z",
      last_seen_at: "2026-10-22T09:00:00Z",
      closed_at: null,
      title_only: true,
      posting_count: 1,
      apply_url: null,
      source_names: ["Example University"],
      posted_at: null,
    },
  ],
};

function mockFetch(routes: Record<string, unknown>) {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = typeof input === "string" ? input : input.toString();
    const match = Object.keys(routes).find((prefix) => url.startsWith(prefix));
    if (!match) return new Response("{}", { status: 404, headers: { "content-type": "application/json" } });
    return new Response(JSON.stringify(routes[match]), { status: 200, headers: { "content-type": "application/json" } });
  });
}

afterEach(() => vi.unstubAllGlobals());

describe("App", () => {
  it("renders loaded opportunities with their provenance and honesty badges", async () => {
    vi.stubGlobal(
      "fetch",
      mockFetch({ "/api/auth/session": SESSION, "/api/stats": STATS, "/api/opportunities": OPPORTUNITIES, "/api/sources": [], "/api/profiles": [] }),
    );
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    // Matches is the default tab now, so Discover is opened explicitly.
    fireEvent.click(screen.getByRole("button", { name: "Discover" }));

    await waitFor(() => expect(screen.getByText("Firmware Engineer, Platform")).toBeDefined());
    expect(screen.getByText("Masterarbeit: Speicherhierarchien")).toBeDefined();
    expect(screen.getByText("2 postings merged")).toBeDefined();
    // A record the source left without a description must say so rather than look like a weak match.
    expect(screen.getByText("No description from source")).toBeDefined();
    // An open topic must never be presented as an ordinary advertised vacancy.
    expect(screen.getByText("Open topic, not a funded post")).toBeDefined();
    expect(screen.getByText("Supervisor Prof. Dr. Anna Beispiel")).toBeDefined();
    expect(screen.getByText(/Deadline in \d+ days, 2026-12-31/)).toBeDefined();
    expect(screen.getByText("No application URL from source")).toBeDefined();
    expect(screen.getByText("4 of 5 sources verified live")).toBeDefined();
    expect(screen.getByText("1 open topics")).toBeDefined();
    expect(screen.getByText("1 stale")).toBeDefined();
  });

  it("reports an unreachable API instead of rendering an empty result set as success", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify({ detail: "connection refused" }), { status: 500, headers: { "content-type": "application/json" } })),
    );
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // With the API answering nothing, the gate cannot tell whether anyone is signed in. It says so
    // and offers a reload, rather than hanging on "checking" or asking for a password it cannot check.
    await waitFor(() => expect(screen.getAllByRole("alert").length).toBeGreaterThan(0));
    expect(screen.getByText(/Cannot reach the API/)).toBeDefined();
    expect(screen.getByRole("button", { name: "Reload" })).toBeDefined();
    expect(screen.queryByRole("button", { name: "Sign in" })).toBeNull();
  });

  it("distinguishes no matches from no sources and failed sources", async () => {
    vi.stubGlobal(
      "fetch",
      mockFetch({
        "/api/auth/session": SESSION, "/api/stats": STATS,
        "/api/opportunities": { total: 0, page: 1, per_page: 25, items: [] },
        "/api/sources": [],
        "/api/profiles": [],
      }),
    );
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    fireEvent.click(screen.getByRole("button", { name: "Discover" }));
    await waitFor(() => expect(screen.getByText(/No opportunity matches these filters/)).toBeDefined());
  });
});
