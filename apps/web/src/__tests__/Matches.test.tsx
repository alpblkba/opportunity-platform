import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "../App";

const SESSION = { email: "owner@example.test", display_name: "Alp", role: "owner", via: "password" };


const STATS = {
  sources_total: 14,
  sources_enabled: 13,
  sources_verified_live: 12,
  opportunities_open: 1848,
  opportunities_closed: 0,
  postings_open: 1902,
  organizations: 14,
  last_complete_sweep_at: "2026-10-22T09:00:00Z",
  sources_stale: 0,
  sources_in_backoff: 0,
  open_topics: 2,
  next_scheduled_run_at: "2026-10-22T15:00:00Z",
};

const PROFILE = {
    id: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
    key: "architecture",
    display_name: "Computer architecture and memory systems",
    priority: 20,
    enabled: true,
    job_kinds: ["employment"],
    terms: ["DRAM"],
    negative_terms: [],
    preferred_technologies: [],
    excluded_technologies: [],
    preferred_countries: ["DE"],
    preferred_cities: [],
    current_country: "DE",
    relocation_willing: true,
    remote_policy: "any",
    include_unknown_location: true,
    min_years_experience: null,
    max_years_experience: null,
    seniority_policy: "preference",
    education_level: null,
    work_authorization: [],
    weights: { terms: 1, concepts: 1, title: 2, location: 1, kind: 1.5, technology: 1, freshness: 0.5, semantic: 2 },
  created_at: "2026-10-22T09:00:00Z",
  updated_at: "2026-10-22T09:00:00Z",
};

const PROFILES = [PROFILE];

const MATCHES = {
  profile_key: "architecture",
  profile_id: PROFILE.id,
  ranking: "lexical ranking only: no embedding provider is configured",
  semantic: { state: "disabled" },
  total_considered: 1848,
  eligible: 1,
  excluded: 1,
  items: [
    {
      id: "11111111-1111-1111-1111-111111111111",
      canonical_title: "Systems Software Engineer, Tiered Memory",
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
      posting_count: 1,
      apply_url: "https://example.org/jobs/1",
      source_names: ["Example Silicon"],
      posted_at: null,
      description_text: "Design page migration policies across CXL-attached capacity.",
      department: null,
      match: {
        score: 68,
        eligible: true,
        exclusions: [],
        factors: [
          {
            name: "related wording",
            contribution: 2,
            detail: "1 related phrase matched even though your exact terms do not appear",
            evidence: ["page migration (your term: DRAM)"],
          },
        ],
        uncertainties: [],
        matched_concepts: ["memory systems"],
        model: "lexical-concepts-v1",
      },
    },
    {
      id: "22222222-2222-2222-2222-222222222222",
      canonical_title: "Sales Engineer",
      organization: "Blocked Corp",
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
      posting_count: 1,
      apply_url: null,
      source_names: ["Blocked Corp"],
      posted_at: null,
      description_text: null,
      department: null,
      match: {
        score: 0,
        eligible: false,
        exclusions: [{ reason: "you excluded the employer Blocked Corp", evidence: "not hiring here", reversible: true }],
        factors: [],
        uncertainties: [{ field: "description", detail: "the source published no description" }],
        matched_concepts: [],
        model: "lexical-concepts-v1",
      },
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

describe("Matches", () => {
  it("ranks opportunities and explains why each one is there", async () => {
    vi.stubGlobal("fetch", mockFetch({ "/api/auth/session": SESSION, "/api/stats": STATS, "/api/profiles": PROFILES, "/api/matches": MATCHES }));
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    fireEvent.click(screen.getByRole("button", { name: "Matches" }));

    await waitFor(() => expect(screen.getByText(/Systems Software Engineer/)).toBeDefined());
    // The score is shown next to the title so the ordering is legible.
    expect(screen.getByText(/68 · Systems Software Engineer/)).toBeDefined();
    // The concept match is explained by naming the phrase that actually hit.
    expect(screen.getByText(/1 related phrase matched/)).toBeDefined();
    expect(screen.getByText(/page migration \(your term: DRAM\)/)).toBeDefined();
  });

  it("states the ranking method and never implies a model ran when none did", async () => {
    vi.stubGlobal("fetch", mockFetch({ "/api/auth/session": SESSION, "/api/stats": STATS, "/api/profiles": PROFILES, "/api/matches": MATCHES }));
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    fireEvent.click(screen.getByRole("button", { name: "Matches" }));

    await waitFor(() => expect(screen.getByText(/lexical ranking only/)).toBeDefined());
    const notice = screen.getByText(/lexical ranking only/).textContent ?? "";
    expect(notice).toContain("no embedding provider is configured");
    expect(notice).toContain("not a probability");
    expect(notice.toLowerCase()).not.toContain("ai-powered");
  });

  it("shows an excluded opportunity with its reason instead of making it vanish", async () => {
    vi.stubGlobal("fetch", mockFetch({ "/api/auth/session": SESSION, "/api/stats": STATS, "/api/profiles": PROFILES, "/api/matches": MATCHES }));
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    fireEvent.click(screen.getByRole("button", { name: "Matches" }));

    await waitFor(() => expect(screen.getByText(/Sales Engineer/)).toBeDefined());
    expect(screen.getByText(/you excluded the employer Blocked Corp/)).toBeDefined();
    expect(screen.getByText(/You can lift this in Profiles/)).toBeDefined();
    // A posting with no link says so rather than rendering a dead control.
    expect(screen.getByText("No application URL from source")).toBeDefined();
  });

  it("distinguishes no matches from no sources", async () => {
    vi.stubGlobal(
      "fetch",
      mockFetch({
        "/api/auth/session": SESSION, "/api/stats": STATS,
        "/api/profiles": PROFILES,
        "/api/matches": { ...MATCHES, items: [], eligible: 0, excluded: 0 },
      }),
    );
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    fireEvent.click(screen.getByRole("button", { name: "Matches" }));
    await waitFor(() => expect(screen.getByText(/not the same as there being no openings/)).toBeDefined());
  });

  it("tells the user what to do when no profile exists yet", async () => {
    vi.stubGlobal("fetch", mockFetch({ "/api/auth/session": SESSION, "/api/stats": STATS, "/api/profiles": [] }));
    // The workspace lives at /workspace; / is the public landing.
    window.history.replaceState({}, "", "/workspace");
    render(<App />);
    // The interface only exists for someone signed in, so wait for that answer first.
    await screen.findByRole("button", { name: "Workspace" });
    fireEvent.click(screen.getByRole("button", { name: "Matches" }));
    await waitFor(() => expect(screen.getByText(/No profile exists yet/)).toBeDefined());
  });
});
