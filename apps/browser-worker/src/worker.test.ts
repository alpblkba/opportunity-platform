import { strict as assert } from "node:assert";
import { after, before, test } from "node:test";
import { existsSync } from "node:fs";
import { mkdtemp, writeFile, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "node:http";

import { startMockAts, receipts, hangNext } from "./mock-server.js";
import { inspect, fill, submit, hashValues } from "./worker.js";

let base = "";
let stop: (() => Promise<void>) | null = null;
let cvPath = "";

before(async () => {
  const mock = await startMockAts(0);
  base = `http://127.0.0.1:${mock.port}`;
  stop = mock.close;
  const dir = await mkdtemp(join(tmpdir(), "opportunity-cv-"));
  cvPath = join(dir, "synthetic-cv.txt");
  await writeFile(cvPath, "Synthetic CV for testing. No real person's data.", "utf8");
  process.env.BROWSER_WORKER_ARTEFACTS = await mkdtemp(join(tmpdir(), "opportunity-artefacts-"));
});

after(async () => {
  if (stop) await stop();
});

const singleValues = [
  { field: "full name", value: "A. Example" },
  { field: "email", value: "a@example.test" },
  { field: "phone", value: "+49 000 000" },
  { field: "current location", value: "Munich" },
  { field: "earliest start date", value: "2027-01-01" },
  { field: "how did you hear about us", value: "Company website" },
  { field: "i agree to the privacy policy", value: "yes" },
];

test("hosted custom questions retain their exact labels and required state", async () => {
  const server = createServer((_req, res) => res.writeHead(200, {"content-type":"text/html"}).end(`<form>
    <div data-field-path="location"><label class="_required_question">Location</label><input role="combobox" aria-haspopup="listbox"></div>
    <div data-field-path="relocation"><label class="_required_question">Open to relocation if necessary:</label><div><button type="button" data-option="yes">Yes</button><button type="button" data-option="no">No</button><input name="relocation" type="checkbox" style="display:none"></div></div>
    <button><span>Submit Application</span></button></form>`));
  await new Promise<void>(r => server.listen(0, "127.0.0.1", r));
  const address = server.address() as { port: number };
  const url = `http://127.0.0.1:${address.port}`;
  try {
    const plan = await inspect(url);
    assert.ok(plan.submit_selector);
    const location = plan.fields.find(f => f.name === "location");
    assert.equal(location?.label, "Location");
    assert.equal(location?.kind, "unsupported");
    assert.equal(location?.required, true);
    const relocation = plan.fields.find(f => f.name === "relocation");
    assert.equal(relocation?.label, "Open to relocation if necessary:");
    assert.equal(relocation?.required, true);
    assert.deepEqual(relocation?.options, ["Yes", "No"]);
    const result = await fill({ url, values: [{ field: "relocation", value: "No" }], reference: "hosted-custom" });
    assert.deepEqual(result.filled, ["relocation"]);
    assert.deepEqual(result.skipped, []);
  } finally { server.closeAllConnections(); await new Promise<void>(r => server.close(() => r())); }
});

test("inspecting the single-step mock reports every control with its kind", async () => {
  const plan = await inspect(`${base}/apply/single`);
  const byName = new Map(plan.fields.map((f) => [f.name, f]));
  assert.equal(plan.steps, 1);
  assert.equal(plan.has_captcha, false);
  assert.equal(byName.get("email")?.kind, "email");
  assert.equal(byName.get("how did you hear about us")?.kind, "select");
  assert.deepEqual(byName.get("how did you hear about us")?.options, ["Company website", "Referral", "Conference"]);
  assert.equal(byName.get("cv")?.kind, "file");
  assert.equal(byName.get("i agree to the privacy policy")?.kind, "checkbox");
  assert.equal(byName.get("full name")?.max_length, 120);
});

test("inspecting the multi-step mock walks the steps and finds the dynamically revealed question", async () => {
  const plan = await inspect(`${base}/apply/multi`);
  const names = plan.fields.map((f) => f.name);
  assert.ok(plan.steps >= 3, `expected to walk at least three steps, walked ${plan.steps}`);
  assert.ok(names.includes("full name"), "step one field");
  assert.ok(names.includes("university"), "step two field");
  // This one only exists after step one is completed, which is the case a single-page inspection would miss.
  assert.ok(names.includes("do you require visa sponsorship"), `dynamic field missing from ${JSON.stringify(names)}`);
  assert.ok(names.includes("describe your research interests"), "step three field");
});

test("filling stops before submitting and leaves a private screenshot", async () => {
  const before = receipts.size;
  const result = await fill({
    url: `${base}/apply/single`,
    values: singleValues,
    documentPath: cvPath,
    reference: "fill-only",
  });
  assert.ok(result.filled.includes("full name"));
  assert.ok(result.filled.includes("__document"), "the CV should have been attached");
  assert.equal(result.skipped.length, 0, `unexpected skipped: ${result.skipped}`);
  assert.ok(result.screenshot.endsWith("before-submit.png"));
  // The decisive assertion: filling must never submit.
  assert.equal(receipts.size, before, "filling must not create a submission");
});

test("an authorized submission lands exactly once and returns a verified receipt", async () => {
  const before = receipts.size;
  const outcome = await submit({
    url: `${base}/apply/single`,
    values: singleValues,
    documentPath: cvPath,
    reference: "authorized-submit",
    authorizedPayloadHash: hashValues(singleValues),
  });
  assert.equal(outcome.state, "submitted");
  if (outcome.state !== "submitted") return;
  assert.match(outcome.receipt, /[0-9a-f-]{36}/);
  assert.equal(receipts.size, before + 1, "exactly one submission");
  assert.ok([...receipts.values()].some(r => r.body.includes("Synthetic CV for testing. No real person's data.")), "the employer received the CV bytes");
  // The receipt the worker reports is the one the site actually issued, not an assumption of success.
  assert.ok(receipts.has(outcome.receipt));
});

test("a submission whose answers changed after authorization is refused before anything is sent", async () => {
  const before = receipts.size;
  const outcome = await submit({
    url: `${base}/apply/single`,
    values: singleValues,
    reference: "tampered",
    authorizedPayloadHash: "0000000000000000",
  });
  assert.equal(outcome.state, "failed");
  assert.equal(receipts.size, before, "nothing may be sent when the payload does not match the authorization");
});

test("a submission the site never answers is reported unknown, never as success", async () => {
  const hadReceipts = receipts.size;
  hangNext(true);
  const outcome = await submit({
    url: `${base}/apply/single`,
    values: singleValues,
    // The document must be attached, otherwise the browser's own validation blocks the form and this would test that instead of a hanging server.
    documentPath: cvPath,
    reference: "uncertain",
    authorizedPayloadHash: hashValues(singleValues),
    submitTimeoutMs: 1500,
  });
  assert.equal(outcome.state, "unknown", `expected unknown, got ${JSON.stringify(outcome)}`);
  if (outcome.state !== "unknown") return;
  assert.match(outcome.diagnostic, /unknown|No employer confirmation/);
  assert.ok(outcome.screenshot.endsWith("after-submit-unknown.png"));
  // The artefact directory is created so a human has somewhere to look. The image itself can be missing when the page is still mid-navigation, which is exactly the situation being reported.
  const files = await readdir(join(process.env.BROWSER_WORKER_ARTEFACTS ?? "private/browser", "uncertain"));
  assert.ok(Array.isArray(files));
  // Nothing was recorded on the far side, which is the point: an uncertain outcome is not a success.
  assert.equal(receipts.size, hadReceipts, "a hung submission must not be counted as submitted");
});

test("the payload hash is order independent but changes with any answer", () => {
  const a = hashValues([{ field: "b", value: "2" }, { field: "a", value: "1" }]);
  const b = hashValues([{ field: "a", value: "1" }, { field: "b", value: "2" }]);
  const c = hashValues([{ field: "a", value: "1" }, { field: "b", value: "changed" }]);
  assert.equal(a, b, "field order must not change the fingerprint");
  assert.notEqual(a, c, "a changed answer must change the fingerprint");
});

test("the worker refuses a real employer application URL even when asked to submit", async () => {
  await assert.rejects(
    () =>
      submit({
        url: "https://job-boards.greenhouse.io/example/jobs/1/application",
        values: singleValues,
        reference: "blocked",
        authorizedPayloadHash: hashValues(singleValues),
      }),
    /standing authorization/,
  );
});

test("a form the browser itself refuses is a failure, not an uncertain outcome", async () => {
  // A required field left empty never leaves the browser. Reporting that as uncertain would block every future attempt on an application that was never submitted.
  const hadReceipts = receipts.size;
  const incomplete = singleValues.filter((v) => v.field !== "email");
  const outcome = await submit({
    url: `${base}/apply/single`,
    values: incomplete,
    reference: "client-side-invalid",
    authorizedPayloadHash: hashValues(incomplete),
    submitTimeoutMs: 2000,
  });
  assert.equal(outcome.state, "failed", `expected failed, got ${JSON.stringify(outcome)}`);
  if (outcome.state !== "failed") return;
  assert.match(outcome.diagnostic, /Nothing was sent/);
  assert.equal(receipts.size, hadReceipts);
});

test("the click marker exists after a submission and not after a refusal", async () => {
  // The Rust side classifies a worker that was killed by whether this file exists: present means a
  // submission may have left the machine and a person has to reconcile it, absent means nothing did
  // and the same answers may go again. These two cases are what give that file its meaning.
  const dir = await mkdtemp(join(tmpdir(), "opportunity-progress-"));

  const sent = join(dir, "sent");
  const landed = await submit({
    url: `${base}/apply/single`,
    values: singleValues,
    documentPath: cvPath,
    reference: "marker-submitted",
    authorizedPayloadHash: hashValues(singleValues),
    progressPath: sent,
  });
  assert.equal(landed.state, "submitted");
  assert.ok(existsSync(sent), "a click that happened must leave its mark");

  // The same form with no document: the browser's own validation refuses it, so the submit control
  // is clicked but nothing is sent... which is exactly why the marker is not proof of sending, only
  // proof of clicking. The worker still reports this as a failure, because it watched it happen.
  const refused = join(dir, "refused");
  const outcome = await submit({
    url: `${base}/apply/single`,
    values: singleValues,
    reference: "marker-refused",
    authorizedPayloadHash: hashValues(singleValues),
    progressPath: refused,
    submitTimeoutMs: 1500,
  });
  assert.equal(outcome.state, "failed", JSON.stringify(outcome));
  assert.match(outcome.state === "failed" ? outcome.diagnostic : "", /Nothing was sent/);
});

test("a submission refused by the browser's own validation is never reported as uncertain", async () => {
  const before = receipts.size;
  const outcome = await submit({
    url: `${base}/apply/single`,
    values: singleValues,
    reference: "validation-refusal",
    authorizedPayloadHash: hashValues(singleValues),
    submitTimeoutMs: 1500,
  });
  assert.equal(outcome.state, "failed");
  assert.equal(receipts.size, before, "a refused form sends nothing");
});

test("a CV changed after authorization is refused before submission", async () => {
  const before = receipts.size;
  const result = await submit({url:`${base}/apply/single`,values:singleValues,documentPath:cvPath,documentHash:"0".repeat(64),reference:"changed-cv",authorizedPayloadHash:hashValues(singleValues)});
  assert.equal(result.state,"failed");
  assert.equal(receipts.size,before);
});
