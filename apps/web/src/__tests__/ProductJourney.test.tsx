import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import CvPanel from "../components/CvPanel";
import SaveApplicationButton from "../components/SaveApplicationButton";
import ApplicationTracking from "../components/ApplicationTracking";
import type { ApplicationRecord } from "../types";

afterEach(() => vi.unstubAllGlobals());
const json = (body: unknown) => new Response(JSON.stringify(body), { headers: { "content-type": "application/json" } });

it("uploads CV bytes and reports storage without promising CV ranking or submission", async () => {
  const cv = { id: "cv-1", label: "synthetic-cv.pdf", version: 1, byte_size: 32 };
  let stored = false;
  const fetch = vi.fn(async (_url: unknown, init?: RequestInit) => {
    if (init?.method === "POST") { stored = true; return json(cv); }
    return json(stored ? [cv] : []);
  });
  vi.stubGlobal("fetch", fetch);
  render(<CvPanel />);
  const file = new File(["%PDF-1.4 synthetic CV"], "synthetic-cv.pdf", { type: "application/pdf" });
  fireEvent.change(screen.getByLabelText(/CV file/), { target: { files: [file] } });
  fireEvent.submit(screen.getByLabelText(/CV file/).closest("form")!);
  await screen.findByRole("link", { name: "synthetic-cv.pdf" });
  expect(screen.getByText(/not semantic CV ranking/)).toBeDefined();
  expect(fetch.mock.calls.find((call) => call[1]?.method === "POST")?.[1]?.body).toBe(file);
  expect(screen.getByRole("status").textContent).toContain("Nothing was submitted");
});

it("saving a result creates a tracked application without preparing or submitting it", async () => {
  const fetch = vi.fn(async (_url: unknown, _init?: RequestInit) => json({ id: "app-1" }));vi.stubGlobal("fetch",fetch);
  render(<SaveApplicationButton opportunityId="opp-1" profileId="profile-1" />);
  fireEvent.click(screen.getByRole("button", { name: "Save to applications" }));
  await screen.findByRole("status");
  expect(fetch.mock.calls[0]?.[0]).toBe("/api/applications");
  expect(screen.getByRole("status").textContent).toContain("Nothing was submitted");
  expect(fetch).toHaveBeenCalledTimes(1);
});

it("shows the tracked employer and records manual progress and notes", async () => {
  const app = { id: "app-1", opportunity_id: "opp-1", status: "saved", notes: null } as ApplicationRecord;
  const onSaved = vi.fn();
  const fetch = vi.fn(async (_url: unknown, init?: RequestInit) => json(init?.method === "POST"
    ? { ...app, status: "interview", notes: "Call on Tuesday" }
    : { opportunity: { canonical_title: "Synthetic firmware role", organization: "Example Silicon", apply_url: "https://example.test/job" }, postings: [] }));
  vi.stubGlobal("fetch",fetch);
  render(<ApplicationTracking application={app} onSaved={onSaved} />);
  await screen.findByText("Synthetic firmware role");
  fireEvent.change(screen.getByLabelText("Recorded status"),{ target: { value: "interview" } });
  fireEvent.change(screen.getByLabelText("Application notes"),{ target: { value: "Call on Tuesday" } });
  fireEvent.click(screen.getByRole("button",{ name: "Save tracking" }));
  await waitFor(() => expect(onSaved).toHaveBeenCalled());
  const posted = fetch.mock.calls.find((call) => call[1]?.method === "POST");
  expect(posted?.[0]).toBe("/api/applications/app-1/tracking");
  expect(posted?.[1]?.body).toBe(JSON.stringify({ status: "interview", notes: "Call on Tuesday" }));
  expect(screen.getByRole("status").textContent).toContain("Nothing was submitted");
});
