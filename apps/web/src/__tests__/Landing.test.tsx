import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import App from "../App";

const home = { cv: null, cv_ready: false, profiles: [{ id: "memory", name: "Memory and operating systems" }], selected_profile: "memory", jobs: [], matches: 0, discovered: 0, applications: [], source_health:{enabled:3,failing:0,never_succeeded:0,stale:0,last_success_at:null},application_counts:{confirmed:0, reported:0, uncertain:0, failed:0, needs_review:0, queued:0, manual:0}, questions: [], policies: [], mail: [], email_configured: false, last_updated: null };
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

/** A server that answers the session check with `session` (null = nobody), and serves the workspace. */
function server(session: null | { via: string }, calls: string[] = [], needs = "native") {
  let signedIn = session;
  return vi.fn(async (input: RequestInfo | URL) => {
    const url = typeof input === "string" ? input : input.toString();
    calls.push(url);
    if (url === "/api/auth/login") { signedIn = { via: "password" }; return json({}); }
    if (url === "/api/auth/logout") { signedIn = null; return json({}); }
    if (!signedIn) return json({ error: "unauthorized", needs, detail: needs === "native" ? "no session" : "no access assertion" }, 401);
    if (url === "/api/auth/session") return json({ email: "owner@example.test", display_name: "Alp", ...signedIn });
    if (url === "/api/home") return json(home);
    return json({});
  });
}

beforeEach(() => window.history.replaceState({}, "", "/"));
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

describe("the public landing page", () => {
  it("says exactly what it is meant to say", () => {
    vi.stubGlobal("fetch", server(null));
    render(<App />);
    expect(screen.getByRole("heading", { name: "Alp's Opportunity Platform" })).toBeDefined();
    expect(screen.getByText("A personal platform for discovering, evaluating and tracking the things worth applying to.")).toBeDefined();
    // The capitalisation is the joke, so it is the thing to assert.
    expect(screen.getByText("bro casually replacing Linkedin")).toBeDefined();
    expect(screen.queryByText(/LinkedIn/)).toBeNull();
    expect(screen.getByRole("link", { name: "what is this?" }).getAttribute("href")).toBe("https://alpblkba.dev/projects/opportunity-platform/");
    expect(screen.getByRole("link", { name: "back to home" }).getAttribute("href")).toBe("https://alpblkba.dev/");
    expect(screen.getByRole("link", { name: "Log in as another user" }).getAttribute("href")).toBe("https://www.youtube.com/watch?v=dQw4w9WgXcQ");
  });

  it("carries the supplied meme, undistorted and described", () => {
    vi.stubGlobal("fetch", server(null));
    render(<App />);
    const meme = screen.getByRole("img");
    expect(meme.getAttribute("src")).toBe("/linkedin-vibe.webp");
    expect(meme.getAttribute("width")).toBe("461");
    expect(meme.getAttribute("height")).toBe("565");
    expect((meme.getAttribute("alt") ?? "").length).toBeGreaterThan(30);
  });

  it("asks the server for nothing private, even for a signed-in owner", async () => {
    const calls: string[] = [];
    vi.stubGlobal("fetch", server({ via: "password" }, calls));
    render(<App />);
    await waitFor(() => expect(screen.getByRole("link", { name: "Log in as Alp (alpblkba)" }).getAttribute("href")).toBe("/workspace"));
    expect(window.location.pathname).toBe("/");
    expect(calls.every((url) => url === "/api/auth/session")).toBe(true);
  });

  it("goes to the login form, signs in to /workspace, and back returns to the landing", async () => {
    vi.stubGlobal("fetch", server(null));
    render(<App />);
    fireEvent.click(screen.getByRole("link", { name: "Log in as Alp (alpblkba)" }));
    fireEvent.change(await screen.findByLabelText("Email"), { target: { value: "owner@example.test" } });
    expect(window.location.pathname).toBe("/login");
    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "synthetic-password" } });
    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
    await screen.findByLabelText("Searching for");
    expect(window.location.pathname).toBe("/workspace");

    fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
    await screen.findByText("bro casually replacing Linkedin");
    expect(window.location.pathname).toBe("/");
    fireEvent.click(screen.getByRole("link", { name: "Log in as Alp (alpblkba)" }));
    await screen.findByLabelText("Password");
    // Signing out is not a session that ran out.
    expect(screen.queryByText(/session ran out/)).toBeNull();
    window.history.back();
    await screen.findByText("bro casually replacing Linkedin");
  });

  it("takes a Cloudflare-authenticated visitor straight to the workspace", async () => {
    vi.stubGlobal("fetch", server({ via: "cloudflare-access" }));
    render(<App />);
    await screen.findByLabelText("Searching for");
    expect(window.location.pathname).toBe("/workspace");
    expect(screen.queryByText("bro casually replacing Linkedin")).toBeNull();
  });
});

describe("the workspace boundary", () => {
  it("shows the login form for a deep link with no session, and stays on the link after sign-in", async () => {
    window.history.replaceState({}, "", "/workspace");
    const calls: string[] = [];
    vi.stubGlobal("fetch", server(null, calls));
    render(<App />);
    fireEvent.change(await screen.findByLabelText("Email"), { target: { value: "owner@example.test" } });
    expect(screen.queryByLabelText("Searching for")).toBeNull();
    expect(calls.includes("/api/home")).toBe(false);
    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "synthetic-password" } });
    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
    await screen.findByLabelText("Searching for");
    expect(window.location.pathname).toBe("/workspace");
  });

  it("offers a reload, not a dead-end password form, when the identity layer in front is missing", async () => {
    window.history.replaceState({}, "", "/workspace");
    vi.stubGlobal("fetch", server(null, [], "identity"));
    render(<App />);
    expect(await screen.findByRole("button", { name: "Reload" })).toBeDefined();
    expect(screen.queryByLabelText("Password")).toBeNull();
    expect(screen.getByRole("link", { name: /back to alpblkba\.dev/ }).getAttribute("href")).toBe("https://alpblkba.dev/");
  });

  it("sends the login page's back link to the personal site", async () => {
    window.history.replaceState({}, "", "/login");
    vi.stubGlobal("fetch", server(null));
    render(<App />);
    const back = await screen.findByRole("link", { name: /back to alpblkba\.dev/ });
    expect(back.getAttribute("href")).toBe("https://alpblkba.dev/");
  });
});
