import { strict as assert } from "node:assert";
import { test } from "node:test";

import { assertNavigable, externalNavigationEnabled, NavigationBlocked } from "./guard.js";

test("loopback addresses are reachable", () => {
  for (const url of [
    "http://127.0.0.1:8099/apply",
    "http://localhost:8099/apply",
    "https://127.0.0.1/apply",
  ]) {
    assert.equal(assertNavigable(url, {}).hostname.length > 0, true);
  }
});

test("a real employer application form is refused by default", () => {
  // The exact case the build contract forbids: an external application URL, with nothing enabling it.
  assert.throws(
    () => assertNavigable("https://job-boards.greenhouse.io/example/jobs/123/application", {}),
    (error: unknown) => {
      assert.ok(error instanceof NavigationBlocked);
      assert.match(error.message, /only loopback addresses are reachable/);
      return true;
    },
  );
});

test("other external hosts and non-web schemes are refused too", () => {
  for (const url of ["https://example.com/", "http://10.0.0.5/", "file:///etc/passwd", "not a url"]) {
    assert.throws(() => assertNavigable(url, {}), NavigationBlocked, `${url} should be refused`);
  }
});

test("the gate opens only on the exact owner-supervised value", () => {
  assert.equal(externalNavigationEnabled({}), false);
  assert.equal(externalNavigationEnabled({ BROWSER_WORKER_ALLOW_EXTERNAL: "1" }), false);
  assert.equal(externalNavigationEnabled({ BROWSER_WORKER_ALLOW_EXTERNAL: "true" }), false);
  assert.equal(externalNavigationEnabled({ BROWSER_WORKER_ALLOW_EXTERNAL: "yes" }), false);
  assert.equal(externalNavigationEnabled({ BROWSER_WORKER_ALLOW_EXTERNAL: "owner-supervised" }), true);

  // With the gate open an external URL parses; this is the owner's deliberate act, never an autonomous one.
  const opened = assertNavigable("https://example.com/apply", {
    BROWSER_WORKER_ALLOW_EXTERNAL: "owner-supervised",
  });
  assert.equal(opened.hostname, "example.com");
});
