import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import App from "../App";

const SESSION = { email: "owner@example.test", display_name: "Alp", role: "owner", via: "password" };

/** A server that refuses until the right password arrives, as the real one does. */
function server(password: string, calls: { url: string; method: string; body?: string }[] = []) {
  let signedIn = false;
  return vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input.toString();
    const method = init?.method ?? "GET";
    calls.push({ url, method, body: init?.body as string | undefined });
    const json = (body: unknown, status = 200) =>
      new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

    if (url === "/api/auth/session") {
      return signedIn ? json(SESSION) : json({ error: "unauthorized", needs: "native", detail: "no session" }, 401);
    }
    if (url === "/api/auth/login") {
      const sent = JSON.parse((init?.body as string) ?? "{}");
      if (sent.password !== password) {
        // One answer for a wrong password and an unknown address alike.
        return json({ error: "unauthorized", needs: "native", detail: "that email and password do not match an account" }, 401);
      }
      signedIn = true;
      return json(SESSION);
    }
    if (url === "/api/auth/logout") {
      signedIn = false;
      return json({ status: "signed out" });
    }
    if (url.startsWith("/api/stats")) {
      return signedIn
        ? json({ sources_total: 0, sources_enabled: 0, sources_verified_live: 0, opportunities_open: 0, opportunities_closed: 0, postings_open: 0, organizations: 0, last_complete_sweep_at: null, sources_stale: 0, sources_in_backoff: 0, open_topics: 0, next_scheduled_run_at: null })
        : json({ error: "unauthorized", needs: "native", detail: "no session" }, 401);
    }
    return json(signedIn ? [] : { error: "unauthorized", needs: "native" }, signedIn ? 200 : 401);
  });
}

afterEach(() => vi.unstubAllGlobals());

/** The login form lives at /login now; / is the public landing. */
function at(path: string) {
  window.history.replaceState({}, "", path);
}

async function signIn(password: string) {
  fireEvent.change(await screen.findByLabelText("Email"), { target: { value: "owner@example.test" } });
  fireEvent.change(screen.getByLabelText("Password"), { target: { value: password } });
  fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
}

describe("native login", () => {
  it("shows the login page, not the workspace, to someone with no session", async () => {
    vi.stubGlobal("fetch", server("correct horse battery staple"));
    at("/login");
    render(<App />);

    await screen.findByRole("heading", { name: /Opportunity Platform/ });
    // The decisive assertion: none of the private interface is rendered for an unauthenticated visitor.
    expect(screen.queryByRole("button", { name: "Workspace" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Matches" })).toBeNull();
    // And the fields are the ones a password manager knows how to fill.
    expect(screen.getByLabelText("Email").getAttribute("autocomplete")).toBe("username");
    expect(screen.getByLabelText("Password").getAttribute("autocomplete")).toBe("current-password");
    expect(screen.getByLabelText("Password").getAttribute("type")).toBe("password");
  });

  it("repeats the server's refusal without guessing whether the account exists", async () => {
    vi.stubGlobal("fetch", server("correct horse battery staple"));
    at("/login");
    render(<App />);
    await signIn("wrong");

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/do not match an account/);
    // Nothing that would tell an attacker which half was wrong.
    expect(alert.textContent).not.toMatch(/password is|no such|unknown|exists/i);
    // Still outside.
    expect(screen.queryByRole("button", { name: "Workspace" })).toBeNull();
  });

  it("opens the workspace on the right password and closes it again on sign out", async () => {
    const calls: { url: string; method: string }[] = [];
    vi.stubGlobal("fetch", server("correct horse battery staple", calls));
    at("/login");
    render(<App />);
    await signIn("correct horse battery staple");

    await screen.findByRole("button", { name: "Workspace" });
    expect(screen.getByText("Alp")).toBeDefined();
    // The password went in a POST body, never in a URL where it would reach a log.
    const login = calls.find((call) => call.url === "/api/auth/login");
    expect(login?.method).toBe("POST");
    expect(calls.every((call) => !call.url.includes("password"))).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Workspace" })).toBeNull());
    expect(calls.some((call) => call.url === "/api/auth/logout" && call.method === "POST")).toBe(true);
  });

  it("asks for the password again when the session runs out mid-session", async () => {
    let signedIn = true;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (input: RequestInfo | URL) => {
        const url = typeof input === "string" ? input : input.toString();
        const json = (body: unknown, status = 200) =>
          new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
        if (url === "/api/auth/session") return signedIn ? json(SESSION) : json({ needs: "native", detail: "no session" }, 401);
        if (url.startsWith("/api/stats")) {
          // The session dies between the gate check and the first real request.
          signedIn = false;
          return json({ error: "unauthorized", needs: "native", detail: "that session is no longer valid" }, 401);
        }
        return json({ error: "unauthorized", needs: "native" }, 401);
      }),
    );
    at("/workspace");
    render(<App />);

    const notice = await screen.findByRole("status");
    expect(notice.textContent).toMatch(/session ran out/i);
    expect(screen.getByRole("button", { name: "Sign in" })).toBeDefined();
  });
});

describe("the gate in a deployment that has no passwords", () => {
  it("never shows a password form when the identity layer in front is what is missing", async () => {
    // Cloudflare mode: the server refuses without naming a native login, because there is nothing
    // this application could ask for. A form here would be a dead end for the only person allowed in.
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(JSON.stringify({ error: "unauthorized", needs: "identity", detail: "no access assertion was presented" }), {
          status: 401,
          headers: { "content-type": "application/json" },
        }),
      ),
    );
    // A private address, because that is where a refusal is shown; / is the public landing.
    at("/workspace");
    render(<App />);

    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/assertion/i);
    expect(screen.queryByLabelText("Password")).toBeNull();
    expect(screen.getByRole("button", { name: "Reload" })).toBeDefined();
  });

  it("shows the workspace to a Cloudflare-authenticated owner with no account", async () => {
    // The rollback experience: an identity that came from the layer in front, no password anywhere.
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
    at("/");
    render(<App />);

    await screen.findByRole("button", { name: "Workspace" });
    expect(screen.queryByLabelText("Password")).toBeNull();
  });
});
