#!/usr/bin/env python3
"""Test backup and restore failure handling with disposable synthetic command fixtures."""
import hashlib
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

BASE = pathlib.Path(__file__).resolve().parents[1]


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        (self.root / "scripts").mkdir()
        (self.root / "bin").mkdir()
        for name in ("backup.sh", "restore.sh", "deploy-vm.sh"):
            shutil.copy2(BASE / "scripts" / name, self.root / "scripts" / name)
        docker = self.root / "bin/docker"
        docker.write_text("""#!/usr/bin/env python3
import os, pathlib, sys
a = sys.argv[1:]
with open(os.environ['FIXTURE_LOG'], 'a') as f: f.write(' '.join(a) + '\\n')
if 'pg_dump' in a:
    print('synthetic database payload')
    sys.exit(31 if os.environ.get('FAIL_DUMP') else 0)
if '--services' in a: print('app')
if 'tar' in a:
    print('synthetic private payload')
    sys.exit(32 if os.environ.get('FAIL_TAR') else 0)
if 'psql' in a and any('pg_database' in v for v in a):
    if os.environ.get('FAIL_LOOKUP'): sys.exit(33)
    if os.environ.get('TARGET_EXISTS'): print('1')
""")
        gpg = self.root / "bin/gpg"
        gpg.write_text("""#!/usr/bin/env python3
import os, pathlib, shutil, sys
if os.environ.get('FAIL_GPG'): sys.exit(34)
a = sys.argv[1:]
shutil.copyfile(a[-1], a[a.index('--output') + 1])
""")
        sha = self.root / "bin/shasum"
        sha.write_text("#!/usr/bin/env python3\nimport hashlib, os, pathlib, sys\nif os.environ.get('FAIL_HASH'): sys.exit(35)\np = pathlib.Path(sys.argv[-1])\nprint(hashlib.sha256(p.read_bytes()).hexdigest() + '  ' + str(p))\n")
        for path in (docker, gpg, sha):
            path.chmod(0o755)
        self.log = self.root / "commands.log"
        self.env = dict(os.environ, PATH=str(self.root / "bin") + os.pathsep + os.environ["PATH"],
                        ENV_FILE="", BACKUP_DIR=str(self.root / "backups"),
                        BACKUP_ENCRYPT_TO="synthetic-recipient", BACKUP_REQUIRE_ENCRYPTION="yes",
                        FIXTURE_LOG=str(self.log), TMPDIR=str(self.root))

    def run_script(self, name, *args, **updates):
        return subprocess.run(["bash", "scripts/" + name, *args], cwd=self.root,
                              env=dict(self.env, **updates), capture_output=True, text=True)

    def dump(self, encrypted=False):
        path = self.root / ("fixture.dump.gpg" if encrypted else "fixture.dump")
        path.write_bytes(b"synthetic database payload")
        pathlib.Path(str(path) + ".sha256").write_text(hashlib.sha256(path.read_bytes()).hexdigest() + "  /obsolete/location.dump\n")
        return path

    def test_encryption_failure_fails_and_removes_plaintext(self):
        result = self.run_script("backup.sh", FAIL_GPG="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(list((self.root / "backups").glob("*.dump")))
        self.assertFalse(list((self.root / "backups").glob("*.gpg.tmp")))

    def test_failed_dump_is_not_retained_as_plaintext(self):
        self.assertNotEqual(self.run_script("backup.sh", FAIL_DUMP="1").returncode, 0)
        self.assertFalse(list((self.root / "backups").glob("*.dump")))

    def test_checksum_failure_is_not_a_successful_backup(self):
        self.assertNotEqual(self.run_script("backup.sh", FAIL_HASH="1").returncode, 0)

    def test_private_archive_failure_is_not_a_complete_backup(self):
        self.assertNotEqual(self.run_script("backup.sh", FAIL_TAR="1").returncode, 0)

    def test_encrypted_backup_files_are_owner_only(self):
        result = self.run_script("backup.sh")
        self.assertEqual(result.returncode, 0, result.stderr)
        files = list((self.root / "backups").iterdir())
        self.assertEqual(len(files), 4)
        self.assertTrue(all(p.stat().st_mode & 0o077 == 0 for p in files))

    def test_required_encryption_refuses_missing_recipient(self):
        self.assertNotEqual(self.run_script("backup.sh", BACKUP_ENCRYPT_TO="").returncode, 0)
        self.assertFalse(self.log.exists())

    def test_restore_verifies_the_selected_file_after_transfer(self):
        result = self.run_script("restore.sh", str(self.dump()), "synthetic_restore")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("--single-transaction --exit-on-error", self.log.read_text())

    def test_restore_rejects_corrupt_dump_before_database_access(self):
        dump = self.dump()
        dump.write_bytes(b"corrupted synthetic data")
        self.assertNotEqual(self.run_script("restore.sh", str(dump)).returncode, 0)
        self.assertFalse(self.log.exists())

    def test_restore_rejects_unsafe_or_truncated_database_names(self):
        for target in ('bad";drop database other;--', "bad'name", "x" * 64):
            with self.subTest(target=target):
                self.assertNotEqual(self.run_script("restore.sh", str(self.dump()), target).returncode, 0)
                self.assertFalse(self.log.exists())

    def test_failed_database_lookup_cannot_be_treated_as_absence(self):
        self.assertNotEqual(self.run_script("restore.sh", str(self.dump()), FAIL_LOOKUP="1").returncode, 0)
        self.assertNotIn("create database", self.log.read_text())

    def test_existing_database_is_preserved(self):
        self.assertNotEqual(self.run_script("restore.sh", str(self.dump()), TARGET_EXISTS="1").returncode, 0)
        self.assertNotIn("drop database", self.log.read_text())

    def test_encrypted_restore_removes_temporary_plaintext(self):
        result = self.run_script("restore.sh", str(self.dump(encrypted=True)))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(list(self.root.glob("opportunity-restore.*")))

    def test_manual_rollback_propagates_full_verification_failure(self):
        ssh = self.root / "bin/ssh"
        ssh.write_text('#!/bin/sh\ncase "$2" in *"sh vm-check.sh"*) exit 36;; esac\nexit 0\n')
        ssh.chmod(0o755)
        result = self.run_script("deploy-vm.sh", "--rollback", "synthetic:previous", TRANSPORT="ssh", SSH_HOST="isolated.invalid", REMOTE_DIR="/synthetic")
        self.assertNotEqual(result.returncode, 0)

# These fixtures execute the real scripts and journal mutations; only Docker/SSH are simulated.
import io
import json
import signal
import tarfile
import time

SHA = "b" * 40
OLD_ID = "sha256:" + "a" * 64
NEW_TAG = "opportunity-app:" + SHA + "-amd64"


def archive_fixture(root, revision=SHA, tags=None, arch="amd64"):
    config = json.dumps({"architecture": arch, "os": "linux", "config": {"Labels": {
        "org.opencontainers.image.revision": revision,
        "org.opencontainers.image.title": "opportunity-runtime"}}}).encode()
    identity = "sha256:" + hashlib.sha256(config).hexdigest()
    path = root / ("opportunity-app-" + SHA + "-amd64.tar.gz")
    manifest = json.dumps([{"Config": identity[7:] + ".json", "RepoTags": tags or [NEW_TAG], "Layers": []}]).encode()
    with tarfile.open(path, "w:gz") as out:
        for name, content in [("manifest.json", manifest), (identity[7:] + ".json", config)]:
            member = tarfile.TarInfo(name); member.size = len(content); out.addfile(member, io.BytesIO(content))
    pathlib.Path(str(path) + ".sha256").write_text(hashlib.sha256(path.read_bytes()).hexdigest() + "  " + str(path) + "\n")
    return path, identity


MODEL_DOCKER = r'''#!/usr/bin/env python3
import gzip,json,os,pathlib,sys,time,urllib.parse
p=pathlib.Path(os.environ['MODEL']);s=json.loads(p.read_text());a=sys.argv[1:]
def save(): p.write_text(json.dumps(s))
def image(ref):
    if ref in s['images']: return s['images'][ref]
    for v in s['images'].values():
        if v==ref:return v
    sys.exit(1)
if a[0]=='info': sys.exit(0)
if a[0]=='image':
    if s.get('missing_previous') and a[2].startswith('opportunity-app:rollback-'):sys.exit(1)
    identity=image(a[2]);f=a[-1]
    if f=='{{.Id}}':print(identity)
    elif f=='{{.Architecture}}':print('amd64')
    elif f=='{{.Architecture}} {{.Os}}':print('amd64 linux')
    elif 'revision' in f:print('b'*40 if identity==s['new_id'] else s.get('old_revision','a'*40))
    elif 'title' in f:print('opportunity-runtime')
    else:print('1000')
elif a[0]=='tag':s['images'][a[2]]=image(a[1]);save()
elif a[0]=='load':
    s['images'][s['new_tag']]=('sha256:'+'c'*64) if s.get('wrong_loaded') else s['new_id'];save()
elif a[0]=='buildx':
    s['build_args']=a;s['images'][s['new_tag']]=s['new_id'];save()
elif a[0]=='save':
    sys.stdout.buffer.write(gzip.decompress(pathlib.Path(s['archive']).read_bytes()))
    sys.exit(43 if s.get('save_failure') else 0)
elif a[0]=='run':sys.exit(42 if s.get('run_failure') else 0)
elif a[0]=='compose':
    if 'exec' in a:
        command=a[-1]
        if 'psql -XAt' in command:
            sys.stdin.read()
            if s.get('migrations'):print('\n'.join(s['migrations']))
            sys.exit(5 if s.get('migration_query_failure') else 0)
        if 'pg_dump' in command:
            sys.stdout.buffer.write(b'not a dump' if s.get('bad_dump') else b'PGDMP'+b'synthetic dump '*64);sys.stdout.flush()
            sys.exit(3 if s.get('dump_failure') else 0)
        if 'tar' in a:
            sys.stdout.buffer.write(gzip.compress(b'synthetic private tree'));sys.stdout.flush()
            sys.exit(4 if s.get('tar_failure') else 0)
    elif 'ps' in a:
        if '-q' in a or '-aq' in a: print('app' if a[-1]=='app' else 'db')
        else: print('app Up\ndb Up')
    elif 'up' in a:
        env=(p.parent/'vm/.env.prod').read_text();tag=[v.split('=',1)[1] for v in env.splitlines() if v.startswith('APP_IMAGE=')][0]
        if tag==s['new_tag'] and s.get('pause_up'):
            (p.parent/'paused').touch()
            for _ in range(500):
                if (p.parent/'resume').exists():break
                time.sleep(.01)
        if tag==s['new_tag'] and s.get('compose_timeout'):sys.exit(124)
        if tag==s['new_tag'] and s.get('compose_failure'):sys.exit(41)
        s['running_tag']=tag;s['running_id']=('sha256:'+'d'*64) if tag==s['new_tag'] and s.get('wrong_running') else image(tag);s['running_mode']='native' if tag==s['new_tag'] and s.get('wrong_mode') else s.get('configured','cloudflare');save()
elif a[0]=='inspect':
    f=a[-1]
    if f=='{{.Image}}':print(s['running_id'])
    elif '.Config.Image' in f:print(s['running_tag'])
    elif '.Config.Env' in f:print('ACCESS_MODE='+s.get('running_mode','native'))
    else:print('fixture=0')
elif a[0]=='logs':print('access: '+('native sessions' if s.get('running_mode')=='native' else 'Cloudflare Access')+'\nassisted application is off')
elif a[0]=='ps':print('fixture-tunnel')
elif a[0]=='exec':
    if 'psql' in a:
        if s.get('query_failure'):sys.exit(1)
        print(s.get('owners',1))
    elif 'pg_isready' in a:print('accepting connections')
    elif 'curl' in a:
        path=urllib.parse.urlsplit(a[-1]).path
        if '-w' not in a:print('<!doctype html><title>Fixture</title>')
        elif path=='/health':print(s.get('health','200'))
        elif path=='/ready':
            s['ready_calls']=s.get('ready_calls',0)+1;save()
            print('500' if s['ready_calls']<=s.get('delay',0) else s.get('ready','200'))
        elif path.startswith('/api/'):print(s.get('private_code','401'))
        else:print('401' if s.get('running_mode')=='cloudflare' else '200')
    else:print(s.get('running_mode','cloudflare'))
elif a[0]=='stats':print('fixture 40MiB / 256MiB')
'''
MODEL_SSH = r'''#!/usr/bin/env python3
import json,os,pathlib,subprocess,sys
p=pathlib.Path(os.environ['MODEL']);command=sys.argv[-1]
if 'REMOTE_PROBLEMS=' in command:print('fixture preflight\nREMOTE_PROBLEMS=0');sys.exit(0)
r=subprocess.run(command,shell=True,stdin=sys.stdin,capture_output=True)
s=json.loads(p.read_text())
if s.get('drop_action') and ('-- '+s['drop_action']+' ') in command:
    s['drop_action']='';p.write_text(json.dumps(s));sys.exit(255)
sys.stdout.buffer.write(r.stdout);sys.stderr.buffer.write(r.stderr);sys.exit(r.returncode)
'''
MODEL_CHECK = r'''#!/bin/sh
exec python3 - <<'PY_CHECK'
import json,os,pathlib,sys
p=pathlib.Path(os.environ['MODEL']);s=json.loads(p.read_text());s.setdefault('checks',[]).append(s['running_id']);p.write_text(json.dumps(s))
if s.get('failed_check') and s['running_id']==s['new_id']:sys.exit(1)
if s.get('failed_recovery') and s['running_id']!=s['new_id']:sys.exit(1)
PY_CHECK
'''


class DeploymentTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name); self.vm = self.root / "vm"
        for name in ["scripts", "bin", "vm"]: (self.root / name).mkdir()
        for name in ["deploy-vm.sh", "vm-release.sh", "release-artifact.py", "release-image.sh", "release-gate.py", "backup-stream.py"]:
            shutil.copy2(BASE / "scripts" / name, self.root / "scripts" / name)
        self.archive, self.new_id = archive_fixture(self.root)
        self.state = {"images": {"fixture:old": OLD_ID}, "running_tag": "fixture:old", "running_id": OLD_ID,
                      "running_mode": "cloudflare", "configured": "cloudflare", "new_tag": NEW_TAG,
                      "new_id": self.new_id, "archive": str(self.archive), "checks": []}
        self.state_path = self.root / "state.json"; self.write_state()
        self.env = dict(os.environ, PATH=str(self.root / "bin") + os.pathsep + os.environ["PATH"],
                        MODEL=str(self.state_path), TRANSPORT="ssh", SSH_HOST="fixture.invalid",
                        REMOTE_DIR=str(self.vm), REVIEWED_SHA=SHA, VM_READY_TIMEOUT="1", PLATFORM="linux/amd64", TARGET="runtime", VERSION=SHA, DIRTY="")
        self.env.pop("APP_IMAGE", None)
        commands = {"docker": MODEL_DOCKER, "ssh": MODEL_SSH,
                    "scp": '#!/usr/bin/env python3\nimport shutil,sys\nshutil.copy2(sys.argv[-2],sys.argv[-1].split(":",1)[1])\n',
                    "git": '#!/bin/sh\ncase "$*" in *rev-parse*) printf "'+SHA+'\\n";; *status*) printf "%s" "${DIRTY:-}";; esac\n',
                    "shasum": '#!/usr/bin/env python3\nimport hashlib,pathlib,sys\np=pathlib.Path(sys.argv[-1]);print(hashlib.sha256(p.read_bytes()).hexdigest()+"  "+str(p))\n',
                    "systemctl": '#!/bin/sh\nexit 3\n', "dmesg": '#!/bin/sh\nexit 1\n',
                    "free": '#!/bin/sh\nprintf "Mem: 1024 200 100 0 0 800\\nSwap: 0 0 0\\n"\n',
                    "cut": '#!/bin/sh\ncase "$*" in */proc/loadavg*) echo "0.0 0.0 0.0";; *) exec /usr/bin/cut "$@";; esac\n'}
        for name, content in commands.items():
            path=self.root / "bin" / name; path.write_text(content); path.chmod(0o755)
        for directory in [self.root, self.vm]:
            for name in ["docker-compose.prod.yml", "docker-compose.e2micro.yml"]: (directory/name).write_text("synthetic compose fixture\n")
        (self.root/"scripts/vm-check.sh").write_text(MODEL_CHECK)
        (self.vm/"vm-check.sh").write_text(MODEL_CHECK)
        (self.vm/".env.prod").write_text("APP_IMAGE=fixture:old\nACCESS_MODE=cloudflare\nPOSTGRES_PASSWORD=synthetic-only\n")
        (self.vm/".env.prod").chmod(0o600)

    def write_state(self): self.state_path.write_text(json.dumps(self.state)) if hasattr(self, "state_path") else None
    def flags(self, **flags):
        self.state=json.loads(self.state_path.read_text()); self.state.update(flags); self.write_state()
    def invoke(self, *args, **env):
        return subprocess.run(["bash","scripts/deploy-vm.sh",*(args or [str(self.archive)])],cwd=self.root,
                              env=dict(self.env,**env),capture_output=True,text=True,timeout=30)
    def assert_recovered(self, result):
        self.assertEqual(result.returncode,1,result.stdout+result.stderr)
        s=json.loads(self.state_path.read_text()); self.assertEqual(s['running_id'],OLD_ID)
        self.assertIn(OLD_ID,s['checks']); self.assertFalse((self.vm/'.release-state').exists())
        self.assertIn('ACCESS_MODE=cloudflare',(self.vm/'.env.prod').read_text())

    def test_success_checks_immutable_identity_and_mode(self):
        r=self.invoke();self.assertEqual(r.returncode,0,r.stdout+r.stderr)
        s=json.loads(self.state_path.read_text());self.assertEqual(s['running_id'],self.new_id)
        self.assertGreaterEqual(s['checks'].count(self.new_id),2)
        history=next((self.vm/'.release-history').iterdir());self.assertEqual((history/'previous-id').read_text().strip(),OLD_ID)
        self.assertEqual(s['images'][(history/'rollback-ref').read_text().strip()],OLD_ID)

    def test_failed_start_restores_prior_config_and_full_check(self):
        (self.root/'docker-compose.prod.yml').write_text('changed release configuration\n')
        self.flags(compose_failure=True);self.assert_recovered(self.invoke())
        self.assertEqual((self.vm/'docker-compose.prod.yml').read_text(),'synthetic compose fixture\n')
    def test_mutation_timeout_is_unresolved_until_reconciled(self):
        self.flags(compose_timeout=True);r=self.invoke()
        self.assertEqual(r.returncode,2,r.stdout+r.stderr)
        self.assertTrue((self.vm/'.release-state').exists())
        self.assertEqual(json.loads(self.state_path.read_text())['checks'],[])
        fixed=self.invoke('--reconcile');self.assertEqual(fixed.returncode,0,fixed.stdout+fixed.stderr)
    def test_failed_health_verification_restores_prior_image(self):
        self.flags(failed_check=True);self.assert_recovered(self.invoke())
    def test_wrong_running_mode_is_not_success(self):
        self.flags(wrong_mode=True);self.assert_recovered(self.invoke())
    def test_wrong_loaded_identity_is_rejected(self):
        self.flags(wrong_loaded=True);self.assert_recovered(self.invoke())
    def test_wrong_running_immutable_identity_is_rejected(self):
        self.flags(wrong_running=True);self.assert_recovered(self.invoke())
    def test_existing_mutable_tag_cannot_be_replaced(self):
        self.state['images'][NEW_TAG]='sha256:'+'c'*64;self.write_state()
        self.assert_recovered(self.invoke())
        self.assertEqual(json.loads(self.state_path.read_text())['images'][NEW_TAG],'sha256:'+'c'*64)
    def test_override_mismatch_rejected_before_remote_mutation(self):
        r=self.invoke(APP_IMAGE='fixture:unintended');self.assertEqual(r.returncode,1)
        self.assertFalse((self.vm/'.release-state').exists());self.assertFalse((self.vm/'.release-history').exists())
    def test_renamed_archive_is_rejected_before_remote_mutation(self):
        renamed=self.root/'opportunity-app-renamed-amd64.tar.gz';shutil.copy2(self.archive,renamed)
        r=self.invoke(str(renamed));self.assertEqual(r.returncode,1);self.assertFalse((self.vm/'.release-state').exists())
    def test_lost_transport_requires_explicit_reconciliation(self):
        for action in ['begin','load','apply']:
            with self.subTest(action=action):
                self.flags(drop_action=action)
                r=self.invoke();self.assertEqual(r.returncode,2,r.stdout+r.stderr)
                self.assertTrue((self.vm/'.release-state').exists())
                retry=self.invoke();self.assertEqual(retry.returncode,2)
                fixed=self.invoke('--reconcile');self.assertEqual(fixed.returncode,0,fixed.stdout+fixed.stderr)
                self.assertFalse((self.vm/'.release-state').exists())
                # Return to the same baseline without editing a journal.
                self.state=json.loads(self.state_path.read_text());self.state['running_id']=OLD_ID;self.state['running_tag']='fixture:old';self.state['running_mode']='cloudflare';self.write_state()
                (self.vm/'.env.prod').write_text('APP_IMAGE=fixture:old\nACCESS_MODE=cloudflare\nPOSTGRES_PASSWORD=synthetic-only\n')
                (self.vm/'.env.prod').chmod(0o600)
    def test_lost_finish_response_can_be_verified_from_history(self):
        self.flags(drop_action='finish');r=self.invoke();self.assertEqual(r.returncode,2,r.stdout+r.stderr)
        token=next((self.vm/'.release-history').iterdir()).name
        fixed=self.invoke('--reconcile',token);self.assertEqual(fixed.returncode,0,fixed.stdout+fixed.stderr)
    def test_missing_retained_previous_image_is_unresolved(self):
        self.flags(failed_check=True,missing_previous=True);r=self.invoke()
        self.assertEqual(r.returncode,2,r.stdout+r.stderr);self.assertTrue((self.vm/'.release-state').exists())
    def test_failed_recovery_retains_unresolved_journal(self):
        self.flags(failed_check=True,failed_recovery=True);r=self.invoke()
        self.assertEqual(r.returncode,2,r.stdout+r.stderr);self.assertTrue((self.vm/'.release-state').exists())
    def test_manual_rollback_runs_complete_verification(self):
        self.flags(failed_recovery=True);r=self.invoke('--rollback','fixture:old')
        self.assertEqual(r.returncode,2,r.stdout+r.stderr)
        self.assertIn(OLD_ID,json.loads(self.state_path.read_text())['checks'])
    def test_signal_and_concurrent_release_leave_reconcilable_state(self):
        self.flags(pause_up=True)
        p=subprocess.Popen(['bash','scripts/deploy-vm.sh',str(self.archive)],cwd=self.root,env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        def close_process():
            if p.poll() is None: p.kill()
            p.communicate()
        self.addCleanup(close_process)
        deadline=time.monotonic()+10
        while not (self.root/'paused').exists() and time.monotonic()<deadline:time.sleep(.02)
        self.assertTrue((self.root/'paused').exists())
        second=self.invoke();self.assertEqual(second.returncode,2)
        p.send_signal(signal.SIGTERM);(self.root/'resume').touch();out,err=p.communicate(timeout=10)
        self.assertEqual(p.returncode,2,out+err);self.assertTrue((self.vm/'.release-state').exists())
        fixed=self.invoke('--reconcile');self.assertEqual(fixed.returncode,0,fixed.stdout+fixed.stderr)
    def test_sigkill_leaves_durable_state_for_explicit_reconciliation(self):
        self.flags(pause_up=True)
        p=subprocess.Popen(['bash','scripts/deploy-vm.sh',str(self.archive)],cwd=self.root,env=self.env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        def close_process():
            if p.poll() is None:p.kill()
            p.communicate()
        self.addCleanup(close_process)
        deadline=time.monotonic()+10
        while not (self.root/'paused').exists() and time.monotonic()<deadline:time.sleep(.02)
        self.assertTrue((self.root/'paused').exists());p.kill();(self.root/'resume').touch()
        p.communicate(timeout=10);self.assertEqual(p.returncode,-signal.SIGKILL)
        self.assertTrue((self.vm/'.release-state').exists())
        self.assertEqual(self.invoke().returncode,2)
        fixed=self.invoke('--reconcile');self.assertEqual(fixed.returncode,0,fixed.stdout+fixed.stderr)
    def test_non_mode_configuration_edit_is_not_silently_committed(self):
        self.flags(drop_action='apply');self.assertEqual(self.invoke().returncode,2)
        envfile=self.vm/'.env.prod';envfile.write_text(envfile.read_text().replace('synthetic-only','synthetic-changed'))
        r=self.invoke('--reconcile');self.assertEqual(r.returncode,2,r.stdout+r.stderr)
        self.assertIn('synthetic-changed',envfile.read_text())
    def test_operator_edit_is_not_overwritten_by_reconciliation(self):
        self.flags(drop_action='load');self.assertEqual(self.invoke().returncode,2)
        (self.vm/'.env.prod').write_text('APP_IMAGE=fixture:old\nACCESS_MODE=native\n')
        r=self.invoke('--reconcile');self.assertEqual(r.returncode,2,r.stdout+r.stderr)
        self.assertIn('ACCESS_MODE=native',(self.vm/'.env.prod').read_text())
    def test_build_requires_clean_reviewed_commit(self):
        for updates in [{'DIRTY':' M changed\n'},{'REVIEWED_SHA':'a'*40}]:
            r=subprocess.run(['bash','scripts/release-image.sh'],cwd=self.root,env=dict(self.env,**updates),capture_output=True,text=True)
            self.assertNotEqual(r.returncode,0)
            self.assertNotIn('build_args',json.loads(self.state_path.read_text()))
    def test_build_defaults_work_from_outside_repository(self):
        env = dict(self.env)
        for key in ['PLATFORM', 'TARGET']: env.pop(key, None)
        r = subprocess.run(['bash', str(self.root/'scripts/release-image.sh')], cwd=self.vm,
                           env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout+r.stderr)
        self.assertTrue((self.root/'dist'/self.archive.name).exists())
    def test_failed_image_check_stops_before_packaging(self):
        self.flags(run_failure=True)
        r = subprocess.run(['bash', 'scripts/release-image.sh'], cwd=self.root,
                           env=self.env, capture_output=True, text=True)
        self.assertNotEqual(r.returncode, 0, r.stdout+r.stderr)
        self.assertFalse((self.root/'dist'/self.archive.name).exists())
    def test_failed_save_pipeline_cannot_report_success(self):
        self.flags(save_failure=True)
        r = subprocess.run(['bash', 'scripts/release-image.sh'], cwd=self.root,
                           env=self.env, capture_output=True, text=True)
        self.assertNotEqual(r.returncode, 0, r.stdout+r.stderr)
        self.assertNotIn('ready to ship:', r.stdout)
    def test_build_is_amd64_runtime_labeled_and_archive_verified(self):
        r=subprocess.run(['bash','scripts/release-image.sh'],cwd=self.root,env=self.env,capture_output=True,text=True)
        self.assertEqual(r.returncode,0,r.stdout+r.stderr)
        args=json.loads(self.state_path.read_text())['build_args']
        self.assertIn('linux/amd64',args);self.assertIn('runtime',args);self.assertIn('GIT_REVISION='+SHA,args)
        again=subprocess.run(['bash','scripts/release-image.sh'],cwd=self.root,env=self.env,capture_output=True,text=True)
        self.assertNotEqual(again.returncode,0)


class ArtifactTests(unittest.TestCase):
    def test_bad_revision_tag_and_architecture_are_rejected(self):
        for kwargs in [{'revision':'a'*40},{'tags':['opportunity-app:unintended']},{'arch':'arm64'}]:
            with self.subTest(kwargs=kwargs), tempfile.TemporaryDirectory() as tmp:
                path,_=archive_fixture(pathlib.Path(tmp),**kwargs)
                r=subprocess.run(['python3',str(BASE/'scripts/release-artifact.py'),str(path),'--revision',SHA],capture_output=True,text=True)
                self.assertNotEqual(r.returncode,0)


class ReadinessTests(unittest.TestCase):
    setUp = DeploymentTests.setUp
    write_state = DeploymentTests.write_state
    flags = DeploymentTests.flags
    # Run the real checker independently of the deploy fixture's deliberately controllable checker.
    def run_checker(self, **flags):
        ready_timeout=str(flags.pop('ready_timeout',1))
        self.flags(running_mode='native',configured='native',**flags)
        shutil.copy2(BASE/'scripts/vm-check.sh',self.vm/'vm-check.sh')
        (self.vm/'.env.prod').write_text('APP_IMAGE=fixture:old\nACCESS_MODE=native\nPOSTGRES_USER=fixture\nPOSTGRES_DB=fixture\n'+''.join(k+'=synthetic\n' for k in ['POSTGRES_PASSWORD','CF_ACCESS_TEAM_DOMAIN','CF_ACCESS_AUD','ACCESS_ALLOWED_EMAILS','CLOUDFLARE_TUNNEL_TOKEN']))
        (self.vm/'.env.prod').chmod(0o600)
        return subprocess.run(['sh','vm-check.sh'],cwd=self.vm,env=dict(self.env,VM_READY_TIMEOUT=ready_timeout),capture_output=True,text=True,timeout=15)
    def test_native_without_owner_is_still_rejected(self):
        r=self.run_checker(owners=0);self.assertNotEqual(r.returncode,0)
    def test_failed_owner_query_is_still_rejected(self):
        r=self.run_checker(query_failure=True);self.assertNotEqual(r.returncode,0)
    def test_persistent_readiness_failure_is_rejected(self):
        r=self.run_checker(ready='500');self.assertNotEqual(r.returncode,0);self.assertIn('deadline expired',r.stdout)
    def test_delayed_readiness_eventually_passes(self):
        r=self.run_checker(delay=1,ready_timeout=10);self.assertEqual(r.returncode,0,r.stdout+r.stderr);self.assertIn('waiting for startup',r.stdout)
    def test_anonymous_access_is_still_rejected(self):
        r=self.run_checker(private_code='200');self.assertNotEqual(r.returncode,0)
    def test_mode_mismatch_still_fails(self):
        self.flags(running_mode='cloudflare');r=self.run_checker(); # run_checker normally resets the mode
        self.flags(running_mode='cloudflare')
        r=subprocess.run(['sh','vm-check.sh'],cwd=self.vm,env=self.env,capture_output=True,text=True,timeout=15)
        self.assertNotEqual(r.returncode,0)



class GateTests(unittest.TestCase):
    """release-gate.py against a real Git history. Its only production input is what the VM reports
    (image revision label, SQLx checksums), so Actions history (skipped, failed or successful runs of
    other commits) cannot make it believe production runs something else."""
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.repo = pathlib.Path(self.tmp.name) / "repo"; (self.repo / "migrations").mkdir(parents=True)
        self.git("init", "-q", "-b", "master")
        self.migration("0001_base.sql", "create table a (id int);\n"); self.A = self.commit("base")
        (self.repo / "code.txt").write_text("v2\n"); self.B = self.commit("code only")
        self.migration("0002_more.sql", "create table b (id int);\n"); self.C = self.commit("adds a migration")
        self.migration("0001_base.sql", "create table a (id bigint);\n"); self.D = self.commit("edits an applied migration")
        self.git("checkout", "-q", "-b", "side", self.A)
        (self.repo / "side.txt").write_text("x\n"); self.SIDE = self.commit("not on master")
        self.git("checkout", "-q", "master")
    def git(self, *args):
        return subprocess.run(["git", "-c", "user.name=fixture", "-c", "user.email=fixture@example.test", "-C", str(self.repo), *args],
                              check=True, capture_output=True, text=True).stdout.strip()
    def migration(self, name, sql): (self.repo / "migrations" / name).write_text(sql)
    def commit(self, message):
        self.git("add", "-A"); self.git("commit", "-q", "-m", message); return self.git("rev-parse", "HEAD")
    def checksums(self, revision):
        out = {}
        for name in self.git("ls-tree", "--name-only", revision, "migrations/").splitlines():
            body = subprocess.run(["git", "-C", str(self.repo), "show", f"{revision}:{name}"], capture_output=True, check=True).stdout
            out[int(pathlib.Path(name).name.split("_")[0])] = hashlib.sha384(body).hexdigest()
        return out
    def status(self, revision, migrations=None, extra=""):
        migrations = self.checksums(revision) if migrations is None else migrations
        lines = ["Configured image: opportunity-app:fixture", "No pending release journal", f"DEPLOYED_REVISION={revision}",
                 "DEPLOYED_IMAGE=sha256:" + "e" * 64] + [f"MIGRATION={v}:{c}" for v, c in sorted(migrations.items())] + ["MIGRATIONS_END"]
        return extra + "\n".join(lines) + "\n"
    def gate(self, candidate, status, *extra):
        self.git("checkout", "-q", candidate)
        path = pathlib.Path(self.tmp.name) / "status.txt"; path.write_text(status)
        return subprocess.run(["python3", str(BASE / "scripts/release-gate.py"), "--repo", str(self.repo), "--status", str(path),
                               "--candidate", candidate, "--trusted-ref", "master", *extra], capture_output=True, text=True)
    def assert_refused(self, result, reason):
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr); self.assertIn(reason, result.stdout)

    def test_code_only_candidate_deploys(self):
        r = self.gate(self.B, self.status(self.A)); self.assertEqual(r.returncode, 0, r.stdout); self.assertIn("code-only", r.stdout)
    def test_repeated_trigger_for_the_running_revision_is_a_no_op(self):
        self.assertEqual(self.gate(self.B, self.status(self.B)).returncode, 10)
    def test_new_migration_is_never_automatic(self):
        self.assert_refused(self.gate(self.C, self.status(self.B)), "schema change (0002_more.sql)")
    def test_migration_edited_after_production_applied_it(self):
        self.assert_refused(self.gate(self.D, self.status(self.C)), "0001_base.sql changed after production applied it")
    def test_out_of_band_build_production_runs_is_refused(self):
        self.assert_refused(self.gate(self.B, self.status("f" * 40, self.checksums(self.A))), "does not know")
    def test_actions_history_cannot_substitute_for_production_state(self):
        # Production really runs C (two migrations), whatever an Actions run for B claimed.
        self.assert_refused(self.gate(self.B, self.status(self.C)), "does not descend")
    def test_candidate_off_the_trusted_branch_is_refused(self):
        self.assert_refused(self.gate(self.SIDE, self.status(self.A)), "not on master")
    def test_unresolved_or_partial_release_is_refused(self):
        self.assert_refused(self.gate(self.B, "UNRESOLVED release=x phase=started\n"), "unresolved release journal")
    def test_concurrent_release_holding_the_vm_lock_is_refused(self):
        self.assert_refused(self.gate(self.B, "Another release operation is still active; state is unresolved\n"), "did not report its migration history")
    def test_missing_revision_label_is_refused(self):
        self.assert_refused(self.gate(self.B, self.status(self.A).replace(f"DEPLOYED_REVISION={self.A}", "DEPLOYED_REVISION=")), "did not report a revision")
    def test_out_of_order_migration_is_refused(self):
        migrations = self.checksums(self.C); migrations[3] = migrations.pop(2)
        self.migration("0002_more.sql", "create table b (id int);\n")
        self.assert_refused(self.gate(self.C, self.status(self.B, migrations)), "applied migration 3")
    def test_dirty_migrations_and_wrong_checkout_are_refused(self):
        self.git("checkout", "-q", self.B); self.migration("0009_local.sql", "select 1;\n")
        path = pathlib.Path(self.tmp.name) / "status.txt"; path.write_text(self.status(self.A))
        r = subprocess.run(["python3", str(BASE / "scripts/release-gate.py"), "--repo", str(self.repo), "--status", str(path), "--candidate", self.B, "--trusted-ref", "master"], capture_output=True, text=True)
        self.assert_refused(r, "uncommitted changes")
        (self.repo / "migrations/0009_local.sql").unlink()
        r = subprocess.run(["python3", str(BASE / "scripts/release-gate.py"), "--repo", str(self.repo), "--status", str(path), "--candidate", self.A, "--trusted-ref", "master"], capture_output=True, text=True)
        self.assert_refused(r, "not the candidate")

    def recovery_point(self, revision, age_hours=1, verified=True, tamper=False):
        directory = pathlib.Path(self.tmp.name) / f"backup-{age_hours}-{verified}-{tamper}-{revision[:6]}"; directory.mkdir()
        for name in ("database.dump.gpg", "private.tar.gz.gpg"): (directory / name).write_bytes(b"ciphertext " + name.encode())
        import datetime as dt
        created = (dt.datetime.now(dt.timezone.utc) - dt.timedelta(hours=age_hours)).isoformat(timespec="seconds").replace("+00:00", "Z")
        manifest = {"format": 1, "created_at": created, "production_revision": revision,
                    "migrations": {str(k): v for k, v in self.checksums(revision).items()},
                    "files": {n: {"sha256": hashlib.sha256((directory / n).read_bytes()).hexdigest()} for n in ("database.dump.gpg", "private.tar.gz.gpg")}}
        data = json.dumps(manifest).encode(); (directory / "manifest.json").write_bytes(data)
        if verified:
            (directory / "restore-verified.json").write_text(json.dumps({"ok": True, "documents_missing": 0, "manifest_sha256": hashlib.sha256(data).hexdigest()}))
        if tamper: (directory / "database.dump.gpg").write_bytes(b"truncated")
        return directory
    def test_migration_rollout_with_a_verified_fresh_recovery_point(self):
        r = self.gate(self.C, self.status(self.B), "--rollout-manifest", str(self.recovery_point(self.B)))
        self.assertEqual(r.returncode, 0, r.stdout); self.assertIn("migration rollout of 0002_more.sql", r.stdout)
    def test_migration_rollout_refuses_unverified_stale_foreign_or_tampered_backups(self):
        for point, reason in [(self.recovery_point(self.B, verified=False), "restore-rehearsal"),
                              (self.recovery_point(self.B, age_hours=30), "older than 24 hours"),
                              (self.recovery_point(self.A), "different production revision"),
                              (self.recovery_point(self.B, tamper=True), "does not match its manifest")]:
            self.assert_refused(self.gate(self.C, self.status(self.B), "--rollout-manifest", str(point)), reason)


class DeployGateTests(unittest.TestCase):
    """The gate inside the real deploy-vm.sh/vm-release.sh path, on the simulated VM."""
    setUp = DeploymentTests.setUp; write_state = DeploymentTests.write_state
    flags = DeploymentTests.flags; invoke = DeploymentTests.invoke
    def assert_untouched(self, result, code):
        self.assertEqual(result.returncode, code, result.stdout + result.stderr)
        s = json.loads(self.state_path.read_text()); self.assertEqual(s["running_id"], OLD_ID); self.assertEqual(s["checks"], [])
        self.assertEqual(sorted(s["images"]), ["fixture:old"]); self.assertFalse((self.vm / ".release-state").exists())
    def test_redeploying_the_running_revision_changes_nothing(self):
        self.flags(old_revision=SHA); r = self.invoke(); self.assert_untouched(r, 0); self.assertIn("already runs", r.stdout)
    def test_schema_change_is_refused_before_any_mutation(self):
        (self.root / "migrations").mkdir(); (self.root / "migrations/0001_new.sql").write_text("select 1;\n")
        r = self.invoke(); self.assert_untouched(r, 1); self.assertIn("schema change", r.stdout)
    def test_applied_migration_missing_from_candidate_is_refused(self):
        self.flags(migrations=["1:" + "0" * 96]); r = self.invoke(); self.assert_untouched(r, 1); self.assertIn("lacks", r.stdout)
    def test_unreadable_migration_history_is_refused(self):
        self.flags(migration_query_failure=True); r = self.invoke(); self.assert_untouched(r, 1)
    def test_unresolved_journal_refuses_with_unresolved_status(self):
        (self.vm / ".release-state").mkdir(); (self.vm / ".release-state/token").write_text("t\n"); (self.vm / ".release-state/phase").write_text("started\n")
        r = self.invoke(); self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertEqual(json.loads(self.state_path.read_text())["running_id"], OLD_ID)
    def test_gate_mode_reports_without_mutation(self):
        r = self.invoke("--gate"); self.assert_untouched(r, 0); self.assertIn("code-only", r.stdout)
        self.flags(old_revision=SHA); self.assertEqual(self.invoke("--gate").returncode, 10)
    def test_status_reports_revision_and_migrations(self):
        self.flags(migrations=["1:" + "a" * 96]); r = self.invoke("--status")
        self.assertEqual(r.returncode, 0, r.stderr); self.assertIn("DEPLOYED_REVISION=" + "a" * 40, r.stdout)
        self.assertIn("MIGRATION=1:" + "a" * 96, r.stdout); self.assertIn("MIGRATIONS_END", r.stdout)
    def gcloud(self):
        log = self.root / "gcloud.log"
        (self.root / "bin/gcloud").write_text("#!/usr/bin/env python3\nimport os,subprocess,sys\na=sys.argv[1:]\nopen(os.environ['GCLOUD_LOG'],'a').write(' '.join(a)+'\\n')\n"
            "if a[:2]==['auth','list']:print('fixture@example.test');sys.exit(0)\n"
            "if a[:2]==['compute','ssh']:\n    c=a[a.index('--command')+1]\n    if 'REMOTE_PROBLEMS=' in c:print('REMOTE_PROBLEMS=0');sys.exit(0)\n    sys.exit(subprocess.run(c,shell=True).returncode)\n"
            "if a[:2]==['compute','scp']:sys.exit(0)\n")
        (self.root / "bin/gcloud").chmod(0o755); return log
    def test_preflight_is_read_only_and_transfer_check_says_it_writes(self):
        log = self.gcloud(); env = dict(TRANSPORT="iap", GCLOUD_LOG=str(log))
        r = self.invoke("--preflight", **env); self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn("scp", log.read_text()); self.assertNotIn("rm -f", log.read_text())
        r = self.invoke("--check-transfer", **env); self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("compute scp", log.read_text()); self.assertIn("rm -f", log.read_text())


FINGERPRINT = "0123456789ABCDEF0123456789ABCDEF01234567"
FAKE_GPG = r"""#!/usr/bin/env python3
import json,os,sys
a=sys.argv[1:];keys=json.loads(os.environ.get('FAKE_GPG_KEYS','[]'))
if '--list-keys' in a:
    found=[k for k in keys if k['fpr']==a[-1]]
    for k in found:print('pub:%s:4096:1:%s:1700000000:::u:::%s:'%(k.get('validity','u'),k['fpr'][-16:],k.get('caps','scESC')));print('fpr:::::::::%s:'%k['fpr'])
    sys.exit(0 if found else 2)
if '--encrypt' in a:
    data=sys.stdin.buffer.read()
    if os.environ.get('FAKE_GPG_FAIL'):sys.exit(2)
    open(a[a.index('--output')+1],'wb').write(b'ENC'+data)
"""


class BackupTests(unittest.TestCase):
    setUp_deploy = DeploymentTests.setUp; write_state = DeploymentTests.write_state; flags = DeploymentTests.flags
    def setUp(self):
        self.setUp_deploy()
        (self.root / "bin/gpg").write_text(FAKE_GPG); (self.root / "bin/gpg").chmod(0o755)
        self.out = self.root / "backups"
        self.env.update(BACKUP_OUT_DIR=str(self.out), BACKUP_ENCRYPT_TO=FINGERPRINT, FAKE_GPG_KEYS=json.dumps([{"fpr": FINGERPRINT}]))
        self.flags(migrations=["1:" + "a" * 96])
    def fetch(self, **env):
        return subprocess.run(["bash", "scripts/deploy-vm.sh", "--fetch-backup"], cwd=self.root, env=dict(self.env, **env), capture_output=True, text=True, timeout=30)
    def backups(self): return sorted(p.name for p in self.out.iterdir()) if self.out.exists() else []
    def test_complete_backup_has_manifest_checksums_and_production_state(self):
        r = self.fetch(); self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        [name] = self.backups(); self.assertTrue(name.startswith("opportunity-")); directory = self.out / name
        manifest = json.loads((directory / "manifest.json").read_text())
        self.assertEqual(manifest["production_revision"], "a" * 40); self.assertEqual(manifest["migrations"], {"1": "a" * 96})
        self.assertEqual(manifest["recipient"], FINGERPRINT)
        for file in ("database.dump.gpg", "private.tar.gz.gpg"):
            self.assertEqual(hashlib.sha256((directory / file).read_bytes()).hexdigest(), manifest["files"][file]["sha256"])
            self.assertTrue((directory / (file + ".sha256")).read_text().startswith(manifest["files"][file]["sha256"]))
        self.assertEqual(oct(directory.stat().st_mode & 0o777), "0o700")
        verify = subprocess.run(["python3", "scripts/backup-stream.py", "verify", str(directory)], cwd=self.root, capture_output=True, text=True)
        self.assertEqual(verify.returncode, 0, verify.stderr)
        (directory / "private.tar.gz.gpg").write_bytes((directory / "private.tar.gz.gpg").read_bytes()[:-3])
        verify = subprocess.run(["python3", "scripts/backup-stream.py", "verify", str(directory)], cwd=self.root, capture_output=True, text=True)
        self.assertNotEqual(verify.returncode, 0); self.assertIn("truncated", verify.stderr)
        (directory / "private.tar.gz.gpg").unlink()
        verify = subprocess.run(["python3", "scripts/backup-stream.py", "verify", str(directory)], cwd=self.root, capture_output=True, text=True)
        self.assertNotEqual(verify.returncode, 0); self.assertIn("missing", verify.stderr)
    def test_interrupted_or_invalid_streams_leave_nothing_and_keep_earlier_backups(self):
        self.out.mkdir(); earlier = self.out / "opportunity-20260101T000000Z"; earlier.mkdir(); (earlier / "manifest.json").write_text("{}")
        for flag, env in [("dump_failure", {}), ("tar_failure", {}), ("bad_dump", {}), (None, {"FAKE_GPG_FAIL": "1"})]:
            self.flags(dump_failure=False, tar_failure=False, bad_dump=False)
            if flag: self.flags(**{flag: True})
            r = self.fetch(**env); self.assertNotEqual(r.returncode, 0, (flag, r.stdout + r.stderr))
            self.assertEqual(self.backups(), [earlier.name], flag); self.assertEqual((earlier / "manifest.json").read_text(), "{}")
    def test_recipient_must_be_one_valid_encryption_key_named_by_fingerprint(self):
        cases = [("owner@example.test", [], "full fingerprint"), ("ABCDEF0123456789", [], "full fingerprint"),
                 (FINGERPRINT, [], "no public key"), (FINGERPRINT, [{"fpr": FINGERPRINT}, {"fpr": FINGERPRINT}], "exactly one"),
                 (FINGERPRINT, [{"fpr": FINGERPRINT, "validity": "r"}], "revoked"), (FINGERPRINT, [{"fpr": FINGERPRINT, "caps": "scSC"}], "no usable encryption")]
        for recipient, keys, reason in cases:
            r = self.fetch(BACKUP_ENCRYPT_TO=recipient, FAKE_GPG_KEYS=json.dumps(keys))
            self.assertNotEqual(r.returncode, 0, recipient); self.assertIn(reason, r.stderr); self.assertEqual(self.backups(), [])
        r = self.fetch(BACKUP_ENCRYPT_TO=FINGERPRINT.lower()); self.assertEqual(r.returncode, 0, r.stderr)
    def test_attest_requires_every_referenced_cv_with_its_recorded_bytes(self):
        r = self.fetch(); self.assertEqual(r.returncode, 0, r.stderr); directory = self.out / self.backups()[0]
        work = self.root / "work"; work.mkdir()
        cv = b"synthetic cv"; buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
            member = tarfile.TarInfo("private/documents/one.txt"); member.size = len(cv); archive.addfile(member, io.BytesIO(cv))
        (work / "private.tar.gz").write_bytes(buffer.getvalue()); (work / "database.dump").write_bytes(b"PGDMP fixture")
        manifest = json.loads((directory / "manifest.json").read_text())
        manifest["files"]["database.dump.gpg"]["plaintext_sha256"] = hashlib.sha256(b"PGDMP fixture").hexdigest()
        manifest["files"]["private.tar.gz.gpg"]["plaintext_sha256"] = hashlib.sha256(buffer.getvalue()).hexdigest()
        (directory / "manifest.json").write_text(json.dumps(manifest))
        (work / "migrations.txt").write_text("1:" + "a" * 96 + "\n"); (work / "tables.txt").write_text("54\n")
        attest = lambda: subprocess.run(["python3", "scripts/backup-stream.py", "attest", str(directory), str(work)], cwd=self.root, capture_output=True, text=True)
        (work / "documents.tsv").write_text("private/documents/one.txt\t" + hashlib.sha256(cv).hexdigest() + "\nprivate/documents/two.pdf\t" + "0" * 64 + "\n")
        r = attest(); self.assertNotEqual(r.returncode, 0); self.assertIn("two.pdf", r.stderr); self.assertFalse((directory / "restore-verified.json").exists())
        (work / "documents.tsv").write_text("/app/private/documents/one.txt\t" + hashlib.sha256(cv).hexdigest() + "\n")
        r = attest(); self.assertEqual(r.returncode, 0, r.stderr)
        verified = json.loads((directory / "restore-verified.json").read_text())
        self.assertEqual((verified["ok"], verified["documents_checked"], verified["documents_missing"]), (True, 1, 0))
        self.assertEqual(verified["manifest_sha256"], hashlib.sha256((directory / "manifest.json").read_bytes()).hexdigest())
        (work / "migrations.txt").write_text("1:" + "b" * 96 + "\n"); self.assertNotEqual(attest().returncode, 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
