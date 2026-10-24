import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../App";

/** A server where nobody is signed in, which is what a stranger meets. */
function anonymous(calls: string[] = []) {
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = typeof input === "string" ? input : input.toString();
    calls.push(url);
    return new Response(JSON.stringify({ error: "unauthorized", needs: "native", detail: "no session" }), {
      status: 401,
      headers: { "content-type": "application/json" },
    });
  });
}

beforeEach(() => window.history.replaceState({}, "", "/"));
afterEach(() => vi.unstubAllGlobals());

describe("the public landing page", () => {
  it("says exactly what it is meant to say", async () => {
    vi.stubGlobal("fetch", anonymous());
    render(<App />);

    expect(screen.getByRole("heading", { name: "Alp's Opportunity Platform" })).toBeDefined();
    expect(
      screen.getByText(
        "A personal platform for discovering, evaluating and tracking the things worth applying to.",
      ),
    ).toBeDefined();
    // The capitalisation is the joke, so it is the thing to assert.
    expect(screen.getByText("bro casually replacing Linkedin")).toBeDefined();
    expect(screen.queryByText(/LinkedIn/)).toBeNull();
    expect(screen.getByRole("link", { name: "what is this?" })).toBeDefined();
    expect(screen.getByRole("link", { name: "back to home" })).toBeDefined();
  });

  it("sends the two buttons where they are supposed to go", async () => {
    vi.stubGlobal("fetch", anonymous());
    render(<App />);

    const other = screen.getByRole("link", { name: "Log in as another user" });
    expect(other.getAttribute("href")).toBe("https://www.youtube.com/watch?v=dQw4w9WgXcQ");

    expect(screen.getByRole("link", { name: "back to home" }).getAttribute("href")).toBe("https://alpblkba.dev/");
    expect(screen.getByRole("link", { name: "what is this?" }).getAttribute("href")).toBe(
      "https://alpblkba.dev/projects/opportunity-platform/",
    );

    // The primary one is a real link, so it can be opened in a new tab, and it moves within the app.
    const primary = screen.getByRole("link", { name: "Log in as Alp (alpblkba)" });
    expect(primary.getAttribute("href")).toBe("/login");
    fireEvent.click(primary);
    await waitFor(() => expect(screen.getByLabelText("Password")).toBeDefined());
    expect(window.location.pathname).toBe("/login");
  });

  it("carries the supplied meme, undistorted and described", async () => {
    vi.stubGlobal("fetch", anonymous());
    render(<App />);

    const meme = screen.getByRole("img");
    expect(meme.getAttribute("src")).toBe("/linkedin-vibe.webp");
    // Its own dimensions, so the browser reserves the right box and the text is never squashed.
    expect(meme.getAttribute("width")).toBe("461");
    expect(meme.getAttribute("height")).toBe("565");
    expect(meme.getAttribute("alt")).toMatch(/linkedin/i);
    expect((meme.getAttribute("alt") ?? "").length).toBeGreaterThan(30);
  });

  it("asks the server for nothing that belongs to anyone", async () => {
    const calls: string[] = [];
    vi.stubGlobal("fetch", anonymous(calls));
    render(<App />);

    await screen.findByRole("heading", { name: "Alp's Opportunity Platform" });
    // The one call it makes is "is anyone signed in", and no private endpoint is touched.
    expect(calls.every((url) => url === "/api/auth/session")).toBe(true);
    expect(calls.some((url) => url.startsWith("/api/stats") || url.startsWith("/api/applications"))).toBe(false);
    // And nothing private is on the page.
    expect(screen.queryByRole("button", { name: "Workspace" })).toBeNull();
    expect(screen.queryByLabelText("Password")).toBeNull();
  });

  it("stays on the landing for an owner who is already signed in natively", async () => {
    // / is this application's front door. Someone with a session who types the address should see
    // it, not be bounced past it - and the button then takes them straight in.
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === "string" ? input : input.toString();
        const json = (body: unknown) =>
          new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
        if (url === "/api/auth/session")
          return json({ email: "owner@example.test", display_name: "Alp", role: "owner", via: "password" });
        if (url.startsWith("/api/stats"))
          return json({ sources_total: 0, sources_enabled: 0, sources_verified_live: 0, opportunities_open: 0, opportunities_closed: 0, postings_open: 0, organizations: 0, last_complete_sweep_at: null, sources_stale: 0, sources_in_backoff: 0, open_topics: 0, next_scheduled_run_at: null });
        return json([]);
      }),
    );
    render(<App />);

    await screen.findByRole("heading", { name: "Alp's Opportunity Platform" });
    expect(screen.getByText("bro casually replacing Linkedin")).toBeDefined();
    expect(window.location.pathname).toBe("/");

    // No password to type, so the button goes in rather than to the form.
    const primary = screen.getByRole("link", { name: "Log in as Alp (alpblkba)" });
    expect(primary.getAttribute("href")).toBe("/workspace");
    fireEvent.click(primary);
    await screen.findByRole("button", { name: "Workspace" });
    expect(window.location.pathname).toBe("/workspace");
  });

  it("takes a Cloudflare-authenticated visitor straight to the workspace", async () => {
    // The rollback behaviour, unchanged: an identity that came from the layer in front never had a
    // front door at /, and still does not.
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === "string" ? input : input.toString();
        const json = (body: unknown) =>
          new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json" } });
        if (url === "/api/auth/session")
          return json({ email: "owner@example.test", display_name: "owner@example.test", role: "owner", via: "cloudflare-access" });
        if (url.startsWith("/api/stats"))
          return json({ sources_total: 0, sources_enabled: 0, sources_verified_live: 0, opportunities_open: 0, opportunities_closed: 0, postings_open: 0, organizations: 0, last_complete_sweep_at: null, sources_stale: 0, sources_in_backoff: 0, open_topics: 0, next_scheduled_run_at: null });
        return json([]);
      }),
    );
    render(<App />);

    await screen.findByRole("button", { name: "Workspace" });
    expect(window.location.pathname).toBe("/workspace");
    expect(screen.queryByText("bro casually replacing Linkedin")).toBeNull();
  });

  it("shows the login page for a deep link into the workspace with no session", async () => {
    window.history.replaceState({}, "", "/workspace");
    vi.stubGlobal("fetch", anonymous());
    render(<App />);

    await waitFor(() => expect(screen.getByLabelText("Password")).toBeDefined());
    expect(screen.queryByRole("button", { name: "Workspace" })).toBeNull();
  });
});

describe("links back out of the application", () => {
  it("sends the login page's back link to the personal site, not the retired Zola page", async () => {
    window.history.replaceState({}, "", "/login");
    vi.stubGlobal("fetch", anonymous());
    render(<App />);

    const back = await screen.findByRole("link", { name: /back to alpblkba\.dev/ });
    expect(back.getAttribute("href")).toBe("https://alpblkba.dev/");
    // The page it used to point at is being removed with the Zola site.
    expect(back.getAttribute("href")).not.toContain("/opportunities");
  });

  it("sends the unreachable-gate screen's back link to the personal site too", async () => {
    window.history.replaceState({}, "", "/workspace");
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(JSON.stringify({ error: "unauthorized", needs: "identity", detail: "no access assertion" }), {
          status: 401,
          headers: { "content-type": "application/json" },
        }),
      ),
    );
    render(<App />);

    const back = await screen.findByRole("link", { name: /back to alpblkba\.dev/ });
    expect(back.getAttribute("href")).toBe("https://alpblkba.dev/");
    expect(back.getAttribute("href")).not.toContain("/opportunities");
  });
});
