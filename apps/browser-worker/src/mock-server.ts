/**
 * Local mock ATS server.
 *
 * Serves the synthetic application forms and accepts their submissions, so the whole workflow can be exercised without going near a real employer. It binds to loopback only and issues a receipt per submission, which is what the worker verifies rather than assuming success.
 */
import { createServer, type IncomingMessage, type ServerResponse } from "node:http";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { randomUUID } from "node:crypto";

const here = dirname(fileURLToPath(import.meta.url));
const mockDir = join(here, "..", "mock");

/** Receipts issued so far, keyed by id. A submission is only real if one of these exists. */
export const receipts = new Map<string, { at: string; body: string }>();

/**
 * When set, the server accepts the submission but never answers, which is how the uncertain-outcome path is exercised: the worker clicked submit and cannot tell whether it landed.
 */
let hangNextSubmission = false;
export function hangNext(value: boolean): void {
  hangNextSubmission = value;
}

async function serveFile(name: string, response: ServerResponse): Promise<void> {
  try {
    const body = await readFile(join(mockDir, name), "utf8");
    response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
    response.end(body);
  } catch {
    response.writeHead(404).end("not found");
  }
}

async function readBody(request: IncomingMessage): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of request) chunks.push(chunk as Buffer);
  return Buffer.concat(chunks).toString("utf8");
}

export function createMockAts() {
  return createServer(async (request, response) => {
    const url = new URL(request.url ?? "/", "http://127.0.0.1");
    if (request.method === "GET" && url.pathname === "/apply/single") return serveFile("single-step.html", response);
    if (request.method === "GET" && url.pathname === "/apply/multi") return serveFile("multi-step.html", response);
    if (request.method === "GET" && url.pathname === "/apply/open") return serveFile("open-questions.html", response);

    if (request.method === "POST" && url.pathname.startsWith("/submit/")) {
      const body = await readBody(request);
      if (hangNextSubmission) {
        hangNextSubmission = false;
        // Deliberately never respond: the client is left not knowing whether the application was recorded.
        return;
      }
      const id = randomUUID();
      receipts.set(id, { at: new Date().toISOString(), body });
      response.writeHead(200, { "content-type": "text/html; charset=utf-8" });
      response.end(
        `<!doctype html><html><body><h1>Application received</h1><p>Your reference is <span id="receipt">${id}</span>.</p></body></html>`,
      );
      return;
    }

    if (request.method === "GET" && url.pathname === "/receipts") {
      response.writeHead(200, { "content-type": "application/json" });
      response.end(JSON.stringify({ count: receipts.size, ids: [...receipts.keys()] }));
      return;
    }

    response.writeHead(404).end("not found");
  });
}

export async function startMockAts(port = 8099): Promise<{ port: number; close: () => Promise<void> }> {
  const server = createMockAts();
  await new Promise<void>((resolve) => server.listen(port, "127.0.0.1", resolve));
  const address = server.address();
  const actual = typeof address === "object" && address ? address.port : port;
  return {
    port: actual,
    close: () =>
      new Promise<void>((resolve) => {
        // A deliberately hung request holds its socket open, so closing has to drop connections rather than wait for them.
        server.closeAllConnections();
        server.close(() => resolve());
      }),
  };
}

if (process.argv[1] && process.argv[1].endsWith("mock-server.js")) {
  const port = Number(process.env.MOCK_ATS_PORT ?? 8099);
  startMockAts(port).then(({ port: actual }) =>
    console.log(`mock ATS listening on http://127.0.0.1:${actual} (/apply/single, /apply/multi, /apply/open)`),
  );
}
