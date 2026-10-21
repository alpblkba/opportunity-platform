#!/usr/bin/env python3
"""Regression-test hygiene guard, including forced stage and clean clone semantics."""
from __future__ import annotations
import pathlib
import shutil
import subprocess
import tempfile
import unittest

BASE = pathlib.Path(__file__).resolve().parents[1]

class PrivacyGuardTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.repo = pathlib.Path(self.tmp.name)
        (self.repo / 'scripts').mkdir()
        shutil.copy(BASE/'scripts/check_privacy.py', self.repo/'scripts/check_privacy.py')
        shutil.copy(BASE/'.gitignore', self.repo/'.gitignore')
        subprocess.run(['git','init','-q'],cwd=self.repo,check=True)

    def run_guard(self):
        return subprocess.run(['python3','scripts/check_privacy.py'],cwd=self.repo, capture_output=True,text=True)

    def add(self, path, force=False):
        target=self.repo/path
        target.parent.mkdir(parents=True,exist_ok=True)
        target.write_bytes(b'synthetic local test data')
        subprocess.run(['git','add']+(['-f'] if force else [])+[path],cwd=self.repo,check=True, capture_output=True)

    def test_clean_clone_does_not_require_private_directory(self):
        self.assertFalse((self.repo/'private').exists())
        self.assertEqual(self.run_guard().returncode,0)

    def test_normal_public_source_allowed(self):
        self.add('src/main.rs')
        self.assertEqual(self.run_guard().returncode,0)

    def test_private_force_stage_rejected(self):
        self.add('private/secret.txt',force=True)
        self.assertNotEqual(self.run_guard().returncode,0)

    def test_env_force_stage_rejected(self):
        self.add('.env',force=True)
        self.assertNotEqual(self.run_guard().returncode,0)

    def test_real_cv_outside_fixture_rejected(self):
        self.add('documents/my-cv.docx',force=True)
        self.assertNotEqual(self.run_guard().returncode,0)

    def test_synthetic_pdf_fixture_allowed(self):
        self.add('tests/fixtures/synthetic-profile.pdf')
        self.assertEqual(self.run_guard().returncode,0)

    def test_synthetic_docx_fixture_allowed(self):
        self.add('tests/fixtures/sanitized-example.docx')
        self.assertEqual(self.run_guard().returncode,0)

    def test_non_synthetic_named_cv_fixture_rejected(self):
        self.add('tests/fixtures/alp_cv.docx',force=True)
        self.assertNotEqual(self.run_guard().returncode,0)

    def test_crate_local_synthetic_fixture_allowed(self):
        self.add('crates/adapters-academic/tests/fixtures/synthetic-thesis.pdf')
        self.assertEqual(self.run_guard().returncode, 0)

    def test_crate_local_real_cv_rejected(self):
        self.add('crates/adapters-academic/tests/fixtures/alp_cv.pdf', force=True)
        self.assertNotEqual(self.run_guard().returncode, 0)

if __name__ == '__main__':
    unittest.main(verbosity=2)
