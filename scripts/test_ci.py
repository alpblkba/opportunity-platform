#!/usr/bin/env python3
"""Exercise verification failures in disposable copies with synthetic command fixtures."""
from __future__ import annotations

import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

BASE = pathlib.Path(__file__).resolve().parents[1]
PASSING_TAP = "# tests 1\n# pass 1\n# fail 0\n# cancelled 0\n# skipped 0\n# todo 0\n"


class VerificationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        (self.root / "scripts").mkdir()
        (self.root / "apps/web").mkdir(parents=True)
        (self.root / "apps/browser-worker/dist").mkdir(parents=True)
        (self.root / "apps/browser-worker/dist/fixture.test.js").touch()
        for name in ("verify-security.sh", "verify-browser-worker.sh"):
            shutil.copy2(BASE / "scripts" / name, self.root / "scripts" / name)
        for name in ("check_privacy.py", "test_privacy.py", "test_ci.py"):
            (self.root / "scripts" / name).write_text(
                f"import os, sys\nsys.exit(74 if os.environ.get('CI_TEST_PRIVACY_FAIL') == {name!r} else 0)\n"
            )
        tools = self.root / "bin"
        tools.mkdir()
        for name in ("cargo", "npm"):
            path = tools / name
            path.write_text(
                '#!/bin/sh\nset -eu\n'
                f'printf "%s\\n" "{name}:$*" >> "$CI_TEST_LOG"\n'
                f'if [ "${{CI_TEST_{name.upper()}_FAIL:-}}" = "$1" ]; then exit 73; fi\n'
                + ('if [ "$1" = run ] && [ "${CI_TEST_NPM_FAIL:-}" = "$2" ]; then exit 73; fi\n'
                   'if [ "$1" = test ]; then cat "$CI_TEST_TAP"; exit "${CI_TEST_NPM_TEST_STATUS:-0}"; fi\n' if name == "npm" else "")
            )
            path.chmod(0o755)
        self.tap = self.root / "result.tap"
        self.tap.write_text(PASSING_TAP)
        self.log = self.root / "commands.log"
        self.env = dict(os.environ, PATH=str(tools) + os.pathsep + os.environ["PATH"],
                        DATABASE_URL="postgres://synthetic-fixture/unused", VERIFY_ENV_FILE="",
                        CI_TEST_LOG=str(self.log), CI_TEST_TAP=str(self.tap))
        (self.root / ".env").write_text("touch env-was-sourced\nexit 91\n")

    def run_script(self, name="verify-browser-worker.sh", **updates):
        return subprocess.run(["bash", "scripts/" + name], cwd=self.root,
                              env=dict(self.env, **updates), capture_output=True, text=True)

    def test_worker_executes_typecheck_build_and_tests(self):
        self.assertEqual(self.run_script().returncode, 0)
        self.assertEqual(self.log.read_text().splitlines(), [
            "npm:run typecheck", "npm:run build", "npm:test -- --test-reporter=tap"])

    def test_missing_generated_worker_tests_fail(self):
        (self.root / "apps/browser-worker/dist/fixture.test.js").unlink()
        result = self.run_script()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("No compiled browser-worker tests", result.stderr)

    def test_zero_tests_fail_even_when_runner_succeeds(self):
        self.tap.write_text(PASSING_TAP.replace("tests 1", "tests 0").replace("pass 1", "pass 0"))
        self.assertNotEqual(self.run_script().returncode, 0)

    def test_mismatched_test_and_pass_counts_fail(self):
        for total, passed in ((2, 1), (1, 2)):
            with self.subTest(total=total, passed=passed):
                self.tap.write_text(PASSING_TAP.replace("tests 1", f"tests {total}").replace("pass 1", f"pass {passed}"))
                self.assertNotEqual(self.run_script().returncode, 0)

    def test_spec_summary_from_newer_node_is_checked_just_as_strictly(self):
        spec = PASSING_TAP.replace("# ", "\u2139 ")
        self.tap.write_text(spec, encoding="utf-8")
        self.assertEqual(self.run_script().returncode, 0)
        self.tap.write_text(spec.replace("fail 0", "fail 1"), encoding="utf-8")
        self.assertNotEqual(self.run_script().returncode, 0)

    def test_missing_test_summary_fails(self):
        self.tap.write_text("no test summary\n")
        self.assertNotEqual(self.run_script().returncode, 0)

    def test_failed_cancelled_skipped_and_todo_tests_fail(self):
        for key in ("fail", "cancelled", "skipped", "todo"):
            with self.subTest(key=key):
                self.tap.write_text(PASSING_TAP.replace(f"{key} 0", f"{key} 1"))
                self.assertNotEqual(self.run_script().returncode, 0)

    def test_runner_failure_is_not_masked_by_tee(self):
        self.assertNotEqual(self.run_script(CI_TEST_NPM_TEST_STATUS="72").returncode, 0)

    def test_worker_typecheck_and_compilation_failures_propagate(self):
        for command in ("typecheck", "build"):
            with self.subTest(command=command):
                self.assertNotEqual(self.run_script(CI_TEST_NPM_FAIL=command).returncode, 0)

    def test_explicit_environment_does_not_source_local_env(self):
        self.assertEqual(self.run_script("verify-security.sh").returncode, 0)
        self.assertFalse((self.root / "env-was-sourced").exists())
        self.assertEqual(self.log.read_text().splitlines().count("cargo:test --locked --workspace"), 1)

    def test_default_local_env_loading_is_preserved(self):
        self.env.pop("VERIFY_ENV_FILE")
        self.assertEqual(self.run_script("verify-security.sh").returncode, 91)
        self.assertTrue((self.root / "env-was-sourced").exists())

    def test_missing_database_configuration_fails(self):
        self.env.pop("DATABASE_URL")
        self.assertNotEqual(self.run_script("verify-security.sh", ALLOW_SKIP_DB_TESTS="1").returncode, 0)

    def test_rust_preflight_lint_and_test_failures_propagate(self):
        for command in ("run", "fmt", "clippy", "test"):
            with self.subTest(command=command):
                self.assertNotEqual(self.run_script("verify-security.sh", CI_TEST_CARGO_FAIL=command).returncode, 0)

    def test_frontend_typecheck_test_and_build_failures_propagate(self):
        for command in ("typecheck", "test", "build"):
            with self.subTest(command=command):
                self.assertNotEqual(self.run_script("verify-security.sh", CI_TEST_NPM_FAIL=command).returncode, 0)

    def test_privacy_failures_propagate(self):
        for command in ("check_privacy.py", "test_privacy.py", "test_ci.py"):
            with self.subTest(command=command):
                self.assertNotEqual(self.run_script("verify-security.sh", CI_TEST_PRIVACY_FAIL=command).returncode, 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
