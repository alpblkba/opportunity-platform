// A worker that dies, so the Rust side's classification of a dead worker can be tested without
// having to arrange a real out-of-memory kill. It answers `inspect` honestly and then, for
// `submit`, either dies before clicking or writes the marker the real worker writes immediately
// before it clicks and dies after. Nothing here touches a browser.
const fs = require("node:fs");
const path = require("node:path");

const command = process.argv[2];
let input = "";
process.stdin.on("data", (chunk) => (input += chunk));
process.stdin.on("end", () => {
  const request = JSON.parse(input || "{}");

  // Deliberately picky: tests in one binary share a process, so this script is reachable by any
  // test that inspects a form while the crash test is running. Answering only its own URL means
  // those tests get a refusal and behave as they would with no worker at all, rather than being
  // handed a plan they never asked for.
  if (command === "inspect" && !String(request.url ?? "").endsWith("/apply/single")) {
    process.stdout.write(JSON.stringify({ error: `this fake worker only knows /apply/single` }));
    return;
  }

  if (command === "inspect") {
    process.stdout.write(
      JSON.stringify({
        url: request.url,
        fields: [
          { name: "full name", label: "Full name", kind: "text", required: true, options: [], max_length: 120 },
          { name: "email", label: "Email", kind: "email", required: true, options: [], max_length: 160 },
        ],
        steps: 1,
        has_captcha: false,
        requires_login: false,
        submit_selector: "button[type=submit]",
      }),
    );
    return;
  }

  if (command === "submit") {
    if (process.env.FAKE_WORKER_MODE === "die_after_clicking") {
      fs.mkdirSync(path.dirname(request.progressPath), { recursive: true });
      fs.writeFileSync(request.progressPath, new Date().toISOString());
    }
    // Killed: no outcome is reported, which is exactly what the real worker leaves behind when the
    // kernel takes it.
    process.kill(process.pid, "SIGKILL");
    return;
  }

  process.stdout.write(JSON.stringify({ error: `unknown command ${command}` }));
});
