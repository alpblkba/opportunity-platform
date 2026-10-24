/**
 * Command line entry point. The Rust API spawns this with a JSON request on stdin and reads a JSON result from stdout, which keeps the browser in its own process and its own dependency tree.
 */
import { inspect, fill, submit, type FillRequest, type SubmitRequest } from "./worker.js";

async function readStdin(): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of process.stdin) chunks.push(chunk as Buffer);
  return Buffer.concat(chunks).toString("utf8");
}

const command = process.argv[2];

try {
  const input = (await readStdin()).trim();
  const request = input ? JSON.parse(input) : {};
  let result: unknown;
  switch (command) {
    case "inspect":
      result = await inspect(request.url);
      break;
    case "fill":
      result = await fill(request as FillRequest);
      break;
    case "submit":
      result = await submit(request as SubmitRequest);
      break;
    default:
      throw new Error(`unknown command '${command}'. Expected inspect, fill or submit.`);
  }
  process.stdout.write(JSON.stringify(result));
} catch (error) {
  process.stdout.write(JSON.stringify({ error: error instanceof Error ? error.message : String(error) }));
  process.exitCode = 1;
}
