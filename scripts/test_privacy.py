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

    def add(self, path, force=False, content=b'synthetic local test data'):
        target=self.repo/path
        target.parent.mkdir(parents=True,exist_ok=True)
        target.write_bytes(content)
        subprocess.run(['git','add']+(['-f'] if force else [])+['--',path],cwd=self.repo,check=True, capture_output=True)

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

    def test_prod_env_force_stage_rejected(self):
        self.add('.env.prod',force=True)
        self.assertNotEqual(self.run_guard().returncode,0)

    def test_deploy_credential_is_ignored_and_force_stage_rejected(self):
        for path in ('gha-creds-0123abcd.json', 'nested/gha-creds-0123abcd.json'):
            with self.subTest(path=path):
                target = self.repo / path
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text('{"type": "external_account", "synthetic": true}')
                ignored = subprocess.run(['git', 'check-ignore', '-q', '--', path], cwd=self.repo)
                self.assertEqual(ignored.returncode, 0, path)
                self.add(path, force=True)
                result = self.run_guard()
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('credential', result.stdout)
                subprocess.run(['git', 'rm', '-q', '--cached', '--', path], cwd=self.repo, check=True)

    def test_nested_private_force_stage_rejected(self):
        self.add('apps/browser-worker/private/browser/form.png',force=True)
        self.assertNotEqual(self.run_guard().returncode,0)

    def test_sanitized_production_example_allowed(self):
        self.add('.env.prod.example',content=(BASE/'.env.prod.example').read_bytes())
        self.assertEqual(self.run_guard().returncode,0)

    def test_existing_development_example_allowed(self):
        self.add('.env.example',content=(BASE/'.env.example').read_bytes())
        self.assertEqual(self.run_guard().returncode,0)

    def test_filled_in_example_secrets_rejected(self):
        for key in ['POSTGRES_PASSWORD', 'CLOUDFLARE_TUNNEL_TOKEN', 'API_SECRET', 'API_KEY', 'CREDENTIAL']:
            with self.subTest(key=key):
                self.add('.env.prod.example',content=f'{key}=synthetic-test-secret\n'.encode())
                self.assertNotEqual(self.run_guard().returncode,0)

    def test_staged_secret_rejected_when_worktree_sanitized(self):
        self.add('.env.prod.example',content=b'TOKEN=synthetic-test-secret\n')
        (self.repo/'.env.prod.example').write_text('TOKEN=REPLACE_WITH_TOKEN\n')
        self.assertNotEqual(self.run_guard().returncode,0)

    def test_safe_staged_example_allowed_when_worktree_has_secret(self):
        self.add('.env.prod.example',content=b'TOKEN=REPLACE_WITH_TOKEN\n')
        (self.repo/'.env.prod.example').write_text('TOKEN=synthetic-test-secret\n')
        self.assertEqual(self.run_guard().returncode,0)

    def test_safe_staged_example_allowed_when_worktree_deleted(self):
        self.add('.env.prod.example',content=b'TOKEN=REPLACE_WITH_TOKEN\n')
        (self.repo/'.env.prod.example').unlink()
        self.assertEqual(self.run_guard().returncode,0)

    def test_example_paths_with_spaces_unicode_and_newlines(self):
        self.add('config space/ü\n/.env.prod.example',content=b'TOKEN=synthetic-test-secret\n')
        result=self.run_guard()
        self.assertNotEqual(result.returncode,0)
        self.assertIn('TOKEN has a value',result.stdout)

    def test_database_url_credentials_rejected(self):
        for value in [
            'postgres://example:synthetic-test-secret@db/app',
            'postgresql://example:synthetic%40test%3Asecret@db/app',
            '"postgres://example:synthetic-test-secret@db/app"',
            'postgres://db/app?password=synthetic-test-secret',
            'postgres://db/app?sslpassword=synthetic-test-secret',
            'postgres://opportunity:opportunity_local_dev@remote.example/app',
            'host=db password=synthetic-test-secret',
            'postgres://[invalid',
        ]:
            with self.subTest(value=value):
                self.add('.env.prod.example',content=f'DATABASE_URL={value}\n'.encode())
                self.assertNotEqual(self.run_guard().returncode,0)

    def test_database_url_placeholders_allowed(self):
        for value in [
            '',
            'REPLACE_WITH_DATABASE_URL',
            'postgres://db/app',
            'postgres://example:@db/app',
            'postgres://example:REPLACE_WITH_PASSWORD@db/app',
            'postgresql://example:REPLACE_WITH_PASSWORD@db/app',
            '"postgres://example:REPLACE_WITH_PASSWORD@db/app"',
            'postgres://db/app?password=REPLACE_WITH_PASSWORD',
        ]:
            with self.subTest(value=value):
                self.add('.env.prod.example',content=f'DATABASE_URL={value}\n'.encode())
                self.assertEqual(self.run_guard().returncode,0)

    def test_database_url_uses_staged_content(self):
        path=self.repo/'.env.prod.example'
        self.add('.env.prod.example',content=b'DATABASE_URL=postgres://example:synthetic-test-secret@db/app\n')
        path.write_text('DATABASE_URL=postgres://example:REPLACE_WITH_PASSWORD@db/app\n')
        self.assertNotEqual(self.run_guard().returncode,0)
        self.add('.env.prod.example',content=path.read_bytes())
        path.write_text('DATABASE_URL=postgres://example:synthetic-test-secret@db/app\n')
        self.assertEqual(self.run_guard().returncode,0)

    def test_unreadable_index_blob_fails_closed(self):
        path='.env.prod.example'
        self.add(path,content=b'TOKEN=REPLACE_WITH_TOKEN\n')
        blob=subprocess.run(['git','rev-parse',f':{path}'],cwd=self.repo,check=True,capture_output=True,text=True).stdout.strip()
        subprocess.run(['git','update-index','--force-remove','--',path],cwd=self.repo,check=True)
        subprocess.run(['git','update-index','--index-info'],input=f'100644 {blob} 2\t{path}\n',text=True,cwd=self.repo,check=True)
        result=self.run_guard()
        self.assertNotEqual(result.returncode,0)
        self.assertIn('cannot read the Git index blob',result.stdout)

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
