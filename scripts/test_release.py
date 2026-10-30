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
import importlib.util
import io
import json
import re
import signal
import tarfile
import time

SHA = "b" * 40
OLD_ID = "sha256:" + "a" * 64
NEW_TAG = "opportunity-app:" + SHA + "-amd64"


def archive_fixture(root, revision=SHA, tags=None, arch="amd64", named=SHA):
    config = json.dumps({"architecture": arch, "os": "linux", "config": {"Labels": {
        "org.opencontainers.image.revision": revision,
        "org.opencontainers.image.title": "opportunity-runtime"}}}).encode()
    identity = "sha256:" + hashlib.sha256(config).hexdigest()
    path = root / ("opportunity-app-" + named + "-amd64.tar.gz")
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
    elif 'revision' in f:print(s.get('new_revision','b'*40) if identity==s['new_id'] else s.get('old_revision','a'*40))
    elif 'title' in f:print('opportunity-runtime')
    else:print('1000')
elif a[0]=='tag':s['images'][a[2]]=image(a[1]);save()
elif a[0]=='load':
    if s.get('load_failure'):print('write /var/lib/docker/tmp: no space left on device',file=sys.stderr);sys.exit(1)
    s['images'][s['new_tag']]=('sha256:'+'c'*64) if s.get('wrong_loaded') else s['new_id'];save()
elif a[0]=='buildx':
    context=pathlib.Path(a[-1])
    s['context']={str(f.relative_to(context)):oct(f.stat().st_mode&0o777) for f in context.rglob('*') if f.is_file()}
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
            rows=['MIGRATION='+m for m in s.get('migrations',[])]+['MIGRATION_FAILED='+m for m in s.get('failed_migrations',[])]
            if rows:print('\n'.join(rows))
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
        new=s['running_id']==s['new_id']
        if '-w' not in a:print('<!doctype html><title>Fixture</title>')
        elif path=='/health':print(s.get('health_new','200') if new else s.get('health','200'))
        elif path=='/ready':
            s['ready_calls']=s.get('ready_calls',0)+1;save()
            print('500' if s['ready_calls']<=s.get('delay',0) else (s.get('ready_new','200') if new else s.get('ready','200')))
        elif path.startswith('/api/'):print(s.get('private_new','401') if new else s.get('private_code','401'))
        else:print('401' if s.get('running_mode')=='cloudflare' else '200')
    else:print(s.get('running_mode','cloudflare'))
elif a[0]=='stats':print('fixture 40MiB / 256MiB')
'''
MODEL_SSH = r'''#!/usr/bin/env python3
import json,os,pathlib,subprocess,sys
p=pathlib.Path(os.environ['MODEL']);command=sys.argv[-1]
if os.environ.get('SSH_LOG'):open(os.environ['SSH_LOG'],'a').write(command+'\0')
if 'REMOTE_PROBLEMS=' in command:print('fixture preflight\nREMOTE_PROBLEMS=0');sys.exit(0)
r=subprocess.run(command,shell=True,executable='/bin/bash',stdin=sys.stdin,capture_output=True)
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

# Copies like scp, or cuts a release archive short (interrupted) or damages it (corrupted in transit).
SCP = r'''#!/usr/bin/env python3
import json,os,pathlib,shutil,sys
s=json.loads(pathlib.Path(os.environ['MODEL']).read_text());source,destination=sys.argv[-2],sys.argv[-1].split(':',1)[1]
data=pathlib.Path(source).read_bytes()
if source.endswith('.tar.gz') and s.get('transfer_failure'):
    pathlib.Path(destination).write_bytes(data[:len(data)//2]);print('lost connection',file=sys.stderr);sys.exit(1)
if source.endswith('.tar.gz') and s.get('corrupt_transfer'):
    pathlib.Path(destination).write_bytes(data[:-7]+b'garbage');sys.exit(0)
shutil.copy2(source,destination)
'''
# GitHub's API as `gh api <path>` sees it, from a JSON file: master HEAD, CI runs, and their jobs.
FAKE_GH = r'''#!/usr/bin/env python3
import json,os,sys
f=json.load(open(os.environ['FAKE_GH']));path=sys.argv[-1]
if os.environ.get('GH_LOG'):open(os.environ['GH_LOG'],'a').write(path+'\n')
if f.get('fail'):print('HTTP 502',file=sys.stderr);sys.exit(1)
if '/git/ref/heads/master' in path:print(json.dumps({'object':{'sha':f['head']}}))
elif '/actions/workflows/' in path:print(json.dumps({'workflow_runs':f.get('runs',[])}))
elif '/jobs' in path:print(json.dumps({'jobs':f.get('jobs',{}).get(path.split('/runs/')[1].split('/')[0],[])}))
else:sys.exit(1)
'''
FAKE_SUDO = '#!/bin/sh\n[ "$1" = -n ] && [ "$2" = -u ] && [ "$4" = -H ] || exit 64\nprintf "%s\\n" "$3" >> "$SUDO_LOG"\nshift 4\nexec "$@"\n'


class DeploymentTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name); self.vm = self.root / "vm"
        for name in ["scripts", "bin", "vm"]: (self.root / name).mkdir()
        for name in ["deploy-vm.sh", "vm-release.sh", "release-artifact.py", "release-image.sh", "release-gate.py", "backup-stream.py",
                     "image-check.sh", "release-candidate.py"]:
            shutil.copy2(BASE / "scripts" / name, self.root / "scripts" / name)
        (self.root / "tree").mkdir(); (self.root / "tree/Dockerfile").write_text("FROM scratch\n")
        self.archive, self.new_id = archive_fixture(self.root)
        self.state = {"images": {"fixture:old": OLD_ID}, "running_tag": "fixture:old", "running_id": OLD_ID,
                      "running_mode": "cloudflare", "configured": "cloudflare", "new_tag": NEW_TAG,
                      "new_id": self.new_id, "archive": str(self.archive), "checks": []}
        self.state_path = self.root / "state.json"; self.write_state()
        self.env = dict(os.environ, PATH=str(self.root / "bin") + os.pathsep + os.environ["PATH"],
                        MODEL=str(self.state_path), TRANSPORT="ssh", SSH_HOST="fixture.invalid",
                        REMOTE_DIR=str(self.vm), REVIEWED_SHA=SHA, VM_READY_TIMEOUT="1", PLATFORM="linux/amd64", TARGET="runtime", VERSION=SHA, DIRTY="",
                        FIXTURE_TREE=str(self.root / "tree"), GITHUB_REPOSITORY="fixture/opportunity-platform",
                        FAKE_GH=str(self.root / "gh.json"), SUDO_LOG=str(self.root / "sudo.log"))
        for key in ("APP_IMAGE", "REQUIRE_CURRENT_HEAD", "RELEASE_OUTCOME_FILE", "REMOTE_RUN_AS", "SSH_LOG"): self.env.pop(key, None)
        (self.root / "gh.json").write_text(json.dumps({"head": SHA}))
        commands = {"docker": MODEL_DOCKER, "ssh": MODEL_SSH,
                    "scp": SCP, "gh": FAKE_GH, "sudo": FAKE_SUDO,
                    "git": '#!/bin/sh\ncase "$*" in *archive*) tar -cf - -C "$FIXTURE_TREE" .;; *rev-parse*) printf "'+SHA+'\\n";; *status*) printf "%s" "${DIRTY:-}";; esac\n',
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

    def outcome(self, **flags):
        self.flags(**flags); path = self.root / "deploy.outcome"; path.unlink(missing_ok=True)
        r = self.invoke(RELEASE_OUTCOME_FILE=str(path))
        fields = {}
        for line in path.read_text().splitlines() if path.exists() else []:
            key, _, value = line.partition("="); fields[key] = value
        return r, fields
    def test_success_reports_deployed_and_keeps_no_archive_in_history(self):
        r, fields = self.outcome(); self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(fields["OUTCOME"], "DEPLOYED"); self.assertEqual(fields["RELEASE_TYPE"], "code-only")
        self.assertEqual(fields["PREVIOUS_REVISION"], "a" * 40); self.assertTrue(fields["ROLLBACK_REF"].startswith("opportunity-app:rollback-"))
        history = next((self.vm / ".release-history").iterdir())
        self.assertEqual(list(history.rglob("*.tar.gz")), [])
        self.assertEqual((history / "archive-sha256").read_text().split()[0], self.archive.with_name(self.archive.name + ".sha256").read_text().split()[0])
    def test_corrupted_transfer_is_caught_on_the_vm_and_rolled_back(self):
        r, fields = self.outcome(corrupt_transfer=True); self.assert_recovered(r)
        self.assertNotIn(NEW_TAG, json.loads(self.state_path.read_text())["images"])
        self.assertEqual((fields["OUTCOME"], fields["ROLLBACK"], fields["STAGE"]), ("ROLLED_BACK", "succeeded", "loading image"))
    def test_interrupted_transfer_is_rolled_back(self):
        r, fields = self.outcome(transfer_failure=True); self.assert_recovered(r)
        self.assertEqual((fields["OUTCOME"], fields["STAGE"]), ("ROLLED_BACK", "transferring"))
    def test_failed_load_such_as_a_full_disk_is_rolled_back(self):
        r, fields = self.outcome(load_failure=True); self.assert_recovered(r)
        self.assertNotIn(NEW_TAG, json.loads(self.state_path.read_text())["images"]); self.assertEqual(fields["STAGE"], "loading image")
    def test_new_release_that_never_becomes_ready_is_rolled_back(self):
        r, fields = self.outcome(ready_new="500"); self.assert_recovered(r)
        self.assertEqual(fields["STAGE"], "starting and verifying the new release"); self.assertIn("Readiness deadline expired", r.stderr)
    def real_checker(self):
        for directory in (self.root / "scripts", self.vm): shutil.copy2(BASE / "scripts/vm-check.sh", directory / "vm-check.sh")
        (self.vm / ".env.prod").write_text("APP_IMAGE=fixture:old\nACCESS_MODE=cloudflare\nPOSTGRES_USER=fixture\nPOSTGRES_DB=fixture\n"
                                           + "".join(k + "=synthetic\n" for k in ["POSTGRES_PASSWORD", "CF_ACCESS_TEAM_DOMAIN", "CF_ACCESS_AUD", "ACCESS_ALLOWED_EMAILS", "CLOUDFLARE_TUNNEL_TOKEN"]))
        (self.vm / ".env.prod").chmod(0o600)
    def test_real_checker_passes_a_healthy_release(self):
        self.real_checker(); r, fields = self.outcome(); self.assertEqual(r.returncode, 0, r.stdout + r.stderr); self.assertEqual(fields["OUTCOME"], "DEPLOYED")
    def assert_really_recovered(self, r, fields, failure):
        # The real checker failed the new release, then passed the restored one in full.
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr); self.assertEqual(fields["OUTCOME"], "ROLLED_BACK")
        s = json.loads(self.state_path.read_text()); self.assertEqual(s["running_id"], OLD_ID); self.assertFalse((self.vm / ".release-state").exists())
        output = r.stdout + r.stderr
        self.assertIn(failure, output); self.assertIn("All checks passed.", output.split(failure, 1)[1])
    def test_real_checker_rolls_back_a_release_whose_health_fails(self):
        # Exactly two failed checks: a count that used to be read as "unresolved" and left it running.
        self.real_checker(); r, fields = self.outcome(health_new="503")
        self.assertIn("2 check(s) failed.", r.stdout); self.assert_really_recovered(r, fields, "/health answered 503")
    def test_real_checker_rolls_back_a_release_that_serves_private_data_anonymously(self):
        self.real_checker(); r, fields = self.outcome(private_new="200")
        self.assert_really_recovered(r, fields, "/api/stats answered 200 to an unauthenticated caller")
    def test_unprovable_recovery_reports_recovery_required(self):
        r, fields = self.outcome(failed_check=True, failed_recovery=True); self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertEqual((fields["OUTCOME"], fields["ROLLBACK"]), ("RECOVERY_REQUIRED", "failed"))
        r, fields = self.outcome(failed_check=False, failed_recovery=False); self.assertEqual(r.returncode, 2)
        self.assertEqual(fields["OUTCOME"], "RECOVERY_REQUIRED"); self.assertEqual(fields["CHANGED"], "no")
    def test_lost_transport_reports_recovery_required_without_rollback(self):
        r, fields = self.outcome(drop_action="apply"); self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertEqual((fields["OUTCOME"], fields["ROLLBACK"]), ("RECOVERY_REQUIRED", "not-attempted"))
    def test_failure_before_any_change_is_reported_as_such(self):
        path = self.root / "deploy.outcome"
        r = self.invoke(str(self.archive), RELEASE_OUTCOME_FILE=str(path), REVIEWED_SHA="a" * 40); self.assertEqual(r.returncode, 1)
        self.assertIn("OUTCOME=FAILED", path.read_text()); self.assertIn("STAGE=artifact verification", path.read_text()); self.assertIn("CHANGED=no", path.read_text())
        self.assertFalse((self.vm / ".release-state").exists())
    def test_commands_run_as_the_owning_user_and_files_travel_over_the_stream(self):
        ssh_log = self.root / "ssh.log"
        r = self.invoke(REMOTE_RUN_AS="fixture_owner", SSH_LOG=str(ssh_log)); self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        commands = [c for c in ssh_log.read_text().split("\0") if c]
        self.assertTrue(commands); self.assertTrue(all(c.startswith("sudo -n -u fixture_owner -H bash -c ") for c in commands), commands[:3])
        self.assertEqual(set((self.root / "sudo.log").read_text().split()), {"fixture_owner"})
        self.assertEqual(json.loads(self.state_path.read_text())["running_id"], self.new_id)
    def test_unsafe_run_as_user_is_refused_before_contact(self):
        ssh_log = self.root / "ssh.log"
        for user in ("root;id", "-u0", "Owner"):
            r = self.invoke(REMOTE_RUN_AS=user, SSH_LOG=str(ssh_log)); self.assertEqual(r.returncode, 2, user); self.assertFalse(ssh_log.exists())
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
        # Nothing named like a release, or half of one, is left behind.
        self.assertEqual(sorted(p.name for p in (self.root / 'dist').iterdir()), [])
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
                 "DEPLOYED_IMAGE=sha256:" + "e" * 64, "CONFIGURED_IMAGE_ID=sha256:" + "e" * 64] + [f"MIGRATION={v}:{c}" for v, c in sorted(migrations.items())] + ["MIGRATIONS_END"]
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
        outcome = pathlib.Path(self.tmp.name) / "gate.outcome"
        r = self.gate(self.C, self.status(self.B), "--outcome-file", str(outcome))
        self.assertEqual(r.returncode, 12, r.stdout + r.stderr); self.assertIn("MIGRATION_REQUIRED: schema change (0002_more.sql)", r.stdout)
        self.assertIn("OUTCOME=MIGRATION_REQUIRED", outcome.read_text()); self.assertIn("PENDING_MIGRATIONS=0002_more.sql", outcome.read_text())
        self.assertIn(f"PREVIOUS_REVISION={self.B}", outcome.read_text())
    def test_candidate_that_is_no_longer_master_head_is_skipped(self):
        outcome = pathlib.Path(self.tmp.name) / "gate.outcome"
        r = self.gate(self.B, self.status(self.A), "--master-head", self.C, "--outcome-file", str(outcome))
        self.assertEqual(r.returncode, 11, r.stdout); self.assertIn("SKIPPED_STALE_REVISION", r.stdout)
        self.assertIn("OUTCOME=SKIPPED_STALE_REVISION", outcome.read_text())
        self.assertEqual(self.gate(self.B, self.status(self.A), "--master-head", self.B).returncode, 0)
        self.assert_refused(self.gate(self.B, self.status(self.A), "--master-head", "HEAD"), "full commit SHA")
    def test_failed_migration_on_production_is_refused(self):
        status = self.status(self.B).replace("MIGRATIONS_END", "MIGRATION_FAILED=2:" + "0" * 96 + "\nMIGRATIONS_END")
        self.assert_refused(self.gate(self.B, status), "failed migration (2)")
    def test_running_image_that_is_not_the_configured_one_is_refused(self):
        status = self.status(self.A).replace("CONFIGURED_IMAGE_ID=sha256:" + "e" * 64, "CONFIGURED_IMAGE_ID=sha256:" + "d" * 64)
        self.assert_refused(self.gate(self.B, status), "out-of-band change")
        self.assert_refused(self.gate(self.B, self.status(self.A).replace("CONFIGURED_IMAGE_ID=sha256:" + "e" * 64 + "\n", "")), "out-of-band change")
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
                              (self.recovery_point(self.B, tamper=True), "does not match its manifest"),
                              (pathlib.Path(self.tmp.name) / "no-such-backup", "no readable backup manifest")]:
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
        outcome = self.root / "deploy.outcome"
        r = self.invoke(RELEASE_OUTCOME_FILE=str(outcome)); self.assert_untouched(r, 12); self.assertIn("schema change", r.stdout)
        self.assertIn("OUTCOME=MIGRATION_REQUIRED", outcome.read_text()); self.assertIn("PENDING_MIGRATIONS=0001_new.sql", outcome.read_text())
    def test_applied_migration_missing_from_candidate_is_refused(self):
        self.flags(migrations=["1:" + "0" * 96]); r = self.invoke(); self.assert_untouched(r, 1); self.assertIn("lacks", r.stdout)
    def test_unreadable_migration_history_is_refused(self):
        self.flags(migration_query_failure=True); r = self.invoke(); self.assert_untouched(r, 1)
    def test_failed_migration_record_is_refused(self):
        self.flags(failed_migrations=["1:" + "0" * 96]); r = self.invoke(); self.assert_untouched(r, 1); self.assertIn("failed migration", r.stdout)
    def test_out_of_band_configured_image_is_refused(self):
        self.state["images"]["fixture:other"] = "sha256:" + "9" * 64; self.write_state()
        (self.vm / ".env.prod").write_text("APP_IMAGE=fixture:other\nACCESS_MODE=cloudflare\nPOSTGRES_PASSWORD=synthetic-only\n")
        r = self.invoke(); self.assertEqual(r.returncode, 1, r.stdout + r.stderr); self.assertIn("out-of-band", r.stdout)
        s = json.loads(self.state_path.read_text()); self.assertEqual(s["running_id"], OLD_ID); self.assertEqual(s["checks"], [])
        self.assertFalse((self.vm / ".release-state").exists())
    def test_a_newer_master_head_stops_the_release_before_any_change(self):
        outcome = self.root / "deploy.outcome"
        (self.root / "gh.json").write_text(json.dumps({"head": "c" * 40}))
        r = self.invoke(REQUIRE_CURRENT_HEAD="yes", RELEASE_OUTCOME_FILE=str(outcome)); self.assert_untouched(r, 11)
        self.assertIn("no longer master HEAD", r.stdout); self.assertIn("OUTCOME=SKIPPED_STALE_REVISION", outcome.read_text())
        (self.root / "gh.json").write_text(json.dumps({"head": SHA, "fail": True}))
        self.assert_untouched(self.invoke(REQUIRE_CURRENT_HEAD="yes"), 1)
    def test_current_master_head_is_checked_and_deploys(self):
        log = self.root / "gh.log"
        r = self.invoke(REQUIRE_CURRENT_HEAD="yes", GH_LOG=str(log)); self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("/git/ref/heads/master", log.read_text())
        self.assertEqual(json.loads(self.state_path.read_text())["running_id"], self.new_id)
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


MARKER = "SYNTHETIC-CREDENTIAL-NOT-A-SECRET"


class ReleaseImageTests(unittest.TestCase):
    """release-image.sh in a real Git checkout, with Docker simulated: what makes it refuse, and what
    the build is handed. google-github-actions/auth writes gha-creds-*.json into this checkout."""
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name); self.repo = self.root / "repo"
        (self.repo / "scripts").mkdir(parents=True); (self.repo / "config").mkdir(); (self.root / "bin").mkdir()
        for name in ("release-image.sh", "release-artifact.py", "image-check.sh"): shutil.copy2(BASE / "scripts" / name, self.repo / "scripts" / name)
        for name in (".gitignore", ".dockerignore", "Dockerfile"): shutil.copy2(BASE / name, self.repo / name)
        (self.repo / "config/app.toml").write_text("synthetic = true\n")
        self.git("init", "-q", "-b", "master"); self.commit("fixture")
        (self.root / "bin/docker").write_text(MODEL_DOCKER); (self.root / "bin/docker").chmod(0o755)
        self.state_path = self.root / "state.json"
        self.env = dict(os.environ, PATH=str(self.root / "bin") + os.pathsep + os.environ["PATH"], MODEL=str(self.state_path),
                        TMPDIR=str(self.root), PLATFORM="linux/amd64", TARGET="runtime")
        for key in ("VERSION", "RELEASE_DIR"): self.env.pop(key, None)
    def git(self, *args):
        return subprocess.run(["git", "-c", "user.name=fixture", "-c", "user.email=fixture@example.test", "-C", str(self.repo), *args],
                              check=True, capture_output=True, text=True).stdout.strip()
    def commit(self, message):
        self.git("add", "-A"); self.git("commit", "-q", "-m", message); self.sha = self.git("rev-parse", "HEAD")
        tag = f"opportunity-app:{self.sha}-amd64"
        (self.root / "fixture").mkdir(exist_ok=True)
        archive, identity = archive_fixture(self.root / "fixture", revision=self.sha, tags=[tag], named=self.sha)
        self.state = {"images": {}, "new_tag": tag, "new_id": identity, "new_revision": self.sha, "archive": str(archive)}
        if hasattr(self, "state_path"): self.state_path.write_text(json.dumps(self.state))
    def build(self, **env):
        self.state_path.write_text(json.dumps(self.state))
        return subprocess.run(["bash", "scripts/release-image.sh"], cwd=self.repo, env=dict(self.env, **{"REVIEWED_SHA": self.sha, **env}),
                              capture_output=True, text=True, timeout=60)
    def plant(self, path, content=MARKER):
        target = self.repo / path; target.parent.mkdir(parents=True, exist_ok=True); target.write_text(content); return target
    def built(self): return json.loads(self.state_path.read_text())
    def assert_refused(self, r, reason):
        self.assertNotEqual(r.returncode, 0, r.stdout + r.stderr); self.assertIn(reason, r.stderr)
        self.assertNotIn("build_args", self.built()); self.assertFalse((self.repo / "dist").exists())

    def test_deploy_credential_neither_dirties_the_checkout_nor_reaches_the_build(self):
        self.plant("gha-creds-0123456789abcdef.json", json.dumps({"type": "external_account", "marker": MARKER}))
        r = self.build(); self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        context = self.built()["context"]
        self.assertIn("Dockerfile", context); self.assertIn("config/app.toml", context)
        self.assertFalse([name for name in context if "gha-creds" in name])
        self.assertNotIn(MARKER, r.stdout + r.stderr)
        self.assertTrue((self.repo / "dist" / f"opportunity-app-{self.sha}-amd64.tar.gz").exists())
        # The extracted context and the build lock are gone again.
        self.assertEqual([p.name for p in self.root.iterdir() if p.name.startswith("opportunity-")], [])
    def test_ignored_local_secrets_never_reach_the_build_either(self):
        for path in (".env.prod", "config/server.key", "config/.env.prod", "private/cv.pdf", "config/gha-creds-nested.json"): self.plant(path)
        r = self.build(); self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(sorted(self.built()["context"]), sorted(self.git("ls-files").splitlines()))
    def test_context_files_keep_the_modes_a_checkout_has(self):
        r = self.build(); self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        context = self.built()["context"]
        self.assertEqual(context["config/app.toml"], "0o644"); self.assertEqual(context["scripts/release-image.sh"], "0o755")
    def test_unexpected_untracked_source_still_refuses(self):
        self.plant("config/stray.toml", "unreviewed = true\n"); self.assert_refused(self.build(), "clean reviewed checkout")
    def test_modified_tracked_file_still_refuses(self):
        self.plant("config/app.toml", "synthetic = false\n"); self.assert_refused(self.build(), "clean reviewed checkout")
    def test_wrong_or_abbreviated_reviewed_sha_still_refuses(self):
        for sha in ("a" * 40, self.sha[:12], ""):
            with self.subTest(sha=sha): self.assert_refused(self.build(REVIEWED_SHA=sha), "complete reviewed HEAD")
    def test_without_the_ignore_rule_the_credential_would_block_every_release(self):
        ignore = self.repo / ".gitignore"
        ignore.write_text("".join(l for l in ignore.read_text().splitlines(keepends=True) if "gha-creds" not in l)); self.commit("drop rule")
        self.plant("gha-creds-0123456789abcdef.json"); self.assert_refused(self.build(), "clean reviewed checkout")
    def test_a_tracked_credential_is_refused_before_the_daemon_sees_it(self):
        self.plant("config/gha-creds-0123456789abcdef.json"); self.git("add", "-f", "config/gha-creds-0123456789abcdef.json"); self.commit("credential")
        self.assert_refused(self.build(), "tracks a deploy credential")
    def test_interrupted_packaging_leaves_nothing_named_like_a_release(self):
        self.state["save_failure"] = True
        r = self.build(); self.assertNotEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(sorted(p.name for p in (self.repo / "dist").iterdir()), [])


def require_docker(test):
    if shutil.which("docker") and subprocess.run(["docker", "info"], capture_output=True).returncode == 0:
        return
    if os.environ.get("CI"):
        test.fail("CI must run the real build-context and image-content checks; Docker is not available")
    test.skipTest("Docker is not available here; CI runs this")


class ContainerContentTests(unittest.TestCase):
    """Real BuildKit and a real container: the patterns and the image check do what they claim."""
    IMAGE = os.environ.get("RELEASE_TEST_IMAGE", "postgres:16-alpine")
    def setUp(self):
        require_docker(self)
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup); self.root = pathlib.Path(self.tmp.name)
    def test_dockerignore_keeps_planted_secrets_out_of_a_real_build_context(self):
        context = self.root / "context"; context.mkdir(); shutil.copy2(BASE / ".dockerignore", context / ".dockerignore")
        kept = ["Dockerfile", "Cargo.toml", "crates/core/src/lib.rs", "config/app.toml", "migrations/0001_base.sql",
                "apps/web/src/main.ts", "apps/web/package.json", ".env.example", ".env.prod.example"]
        excluded = ["gha-creds-0123abcd.json", "config/gha-creds-0123abcd.json", "apps/web/gha-creds-0123abcd.json",
                    ".env", ".env.prod", "config/.env.prod", "apps/web/.env.local", "config/server.key", "apps/web/cert.pem",
                    "crates/core/client.p12", "config/client.pfx", "private/documents/cv.pdf", "apps/browser-worker/private/browser/form.png",
                    "backups/database.dump", "crates/core/old.dump", "config/cv.docx", "apps/web/session.log", "dist/old.tar.gz"]
        for name in kept + excluded:
            (context / name).parent.mkdir(parents=True, exist_ok=True); (context / name).write_text(MARKER)
        probe = self.root / "probe"; probe.mkdir(); (probe / "Dockerfile").write_text("FROM scratch\nCOPY . /context/\n")
        out = self.root / "out"
        r = subprocess.run(["docker", "buildx", "build", "--quiet", "--output", f"type=local,dest={out}", "-f", str(probe / "Dockerfile"), str(context)],
                           capture_output=True, text=True, timeout=180)
        self.assertEqual(r.returncode, 0, r.stderr[-2000:])
        shipped = {str(p.relative_to(out / "context")) for p in (out / "context").rglob("*") if p.is_file()}
        self.assertEqual(sorted(set(kept) - shipped), []); self.assertEqual(sorted(set(excluded) & shipped), [])
        self.assertNotIn(MARKER, r.stdout + r.stderr)
    def check_image(self, plant=None):
        setup = ("mkdir -p /app/apps/web/dist /app/config /app/private/browser && touch /app/apps/web/dist/index.html"
                 " && printf '#!/bin/sh\\necho opportunity fixture\\n' > /usr/local/bin/opportunity && chmod 755 /usr/local/bin/opportunity")
        if plant:
            setup += f" && mkdir -p $(dirname {plant}) && echo {MARKER} > {plant}"
        with open(BASE / "scripts/image-check.sh") as script:
            return subprocess.run(["docker", "run", "--rm", "-i", "--network", "none", "--user", "0", "--entrypoint", "sh", self.IMAGE,
                                   "-c", setup + " && exec sh -s"], stdin=script, capture_output=True, text=True, timeout=120)
    def test_image_check_fails_any_image_carrying_a_planted_secret(self):
        r = self.check_image(); self.assertEqual(r.returncode, 0, r.stdout + r.stderr); self.assertIn("image contents checked", r.stdout)
        for plant in ("/app/config/gha-creds-0123abcd.json", "/root/gha-creds-0123abcd.json", "/app/.env.prod", "/opt/.env",
                      "/app/config/server.key", "/app/config/server.pem", "/app/private/documents/cv.pdf", "/var/backup.dump", "/app/session.log"):
            with self.subTest(plant=plant):
                r = self.check_image(plant); self.assertNotEqual(r.returncode, 0, r.stdout)
                self.assertIn(plant, r.stderr); self.assertNotIn(MARKER, r.stdout + r.stderr)
        # The base image's own certificate bundle is not a leak.
        self.assertEqual(self.check_image("/etc/ssl/certs/fixture-ca.pem").returncode, 0)


REPO = "fixture-owner/opportunity-platform"
JOBS = ["Repository security and privacy", "Rust checks with PostgreSQL", "Product acceptance journey", "Frontend validation", "Browser-worker validation"]


class CandidateTests(unittest.TestCase):
    """release-candidate.py check: which triggers, HEADs and CI results may reach production."""
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name); self.repo = self.root / "repo"; self.repo.mkdir(); (self.root / "bin").mkdir()
        git = lambda *a: subprocess.run(["git", "-c", "user.name=f", "-c", "user.email=f@example.test", "-C", str(self.repo), *a], check=True, capture_output=True, text=True).stdout.strip()
        git("init", "-q", "-b", "master"); (self.repo / "a").write_text("a\n"); git("add", "-A"); git("commit", "-q", "-m", "a")
        self.sha = git("rev-parse", "HEAD")
        (self.root / "bin/gh").write_text(FAKE_GH); (self.root / "bin/gh").chmod(0o755)
        self.api = {"head": self.sha, "runs": [self.ci_run(41)], "jobs": {"41": [{"name": n, "conclusion": "success"} for n in JOBS]}}
        self.event = {"workflow_run": self.ci_run(41)}
    def ci_run(self, run_id, **changes):
        run = {"id": run_id, "head_sha": self.sha, "event": "push", "head_branch": "master", "status": "completed", "conclusion": "success",
               "path": ".github/workflows/hygiene.yml", "repository": {"full_name": REPO}, "head_repository": {"full_name": REPO}}
        run.update(changes); return run
    def check(self, event_name="workflow_run", **env):
        (self.root / "gh.json").write_text(json.dumps(self.api)); (self.root / "event.json").write_text(json.dumps(self.event))
        output = self.root / "github_output"; output.write_text("")
        full = dict(os.environ, PATH=str(self.root / "bin") + os.pathsep + os.environ["PATH"], FAKE_GH=str(self.root / "gh.json"),
                    GITHUB_REPOSITORY=REPO, GITHUB_EVENT_NAME=event_name, GITHUB_EVENT_PATH=str(self.root / "event.json"),
                    GITHUB_REF="refs/heads/master", GITHUB_SHA=self.sha, REVIEWED_SHA=self.sha, GITHUB_OUTPUT=str(output))
        full.update(env)
        r = subprocess.run(["python3", "-I", str(BASE / "scripts/release-candidate.py"), "check"], cwd=self.repo, env=full, capture_output=True, text=True)
        r.outputs = dict(line.split("=", 1) for line in output.read_text().splitlines() if "=" in line)
        return r
    def assert_refused(self, r, reason):
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr); self.assertIn(reason, r.stderr); self.assertEqual(r.outputs.get("outcome"), "REFUSED")

    def test_successful_push_ci_for_the_current_master_head_is_eligible(self):
        r = self.check(); self.assertEqual(r.returncode, 0, r.stdout + r.stderr); self.assertEqual(r.outputs["outcome"], "ELIGIBLE")
    def test_pull_request_ci_never_deploys(self):
        self.event["workflow_run"]["event"] = "pull_request"; self.assert_refused(self.check(), "pull request")
    def test_fork_ci_never_deploys_even_from_a_branch_called_master(self):
        self.event["workflow_run"].update(event="pull_request", head_repository={"full_name": "attacker/opportunity-platform"})
        self.assert_refused(self.check(), "pull request")
        self.event["workflow_run"]["event"] = "push"; self.assert_refused(self.check(), "fork")
    def test_failed_or_cancelled_master_ci_never_deploys(self):
        for conclusion in ("failure", "cancelled", "skipped", None):
            with self.subTest(conclusion=conclusion):
                self.event["workflow_run"]["conclusion"] = conclusion; self.assert_refused(self.check(), "not success")
    def test_another_workflow_or_repository_cannot_stand_in_for_ci(self):
        self.event["workflow_run"]["path"] = ".github/workflows/continuous-integration-lookalike.yml"; self.assert_refused(self.check(), "not continuous integration")
        self.event = {"workflow_run": self.ci_run(41, repository={"full_name": "other/repo"})}; self.assert_refused(self.check(), "another repository")
        self.event = {"workflow_run": self.ci_run(41, head_branch="feature")}; self.assert_refused(self.check(), "another branch")
    def test_an_older_master_commit_is_stale_and_changes_nothing(self):
        self.api["head"] = "c" * 40
        r = self.check(); self.assertEqual(r.returncode, 11, r.stdout + r.stderr)
        self.assertEqual(r.outputs["outcome"], "SKIPPED_STALE_REVISION"); self.assertEqual(r.outputs["master_head"], "c" * 40)
        self.assertIn("No production changes performed.", r.stdout)
    def test_manual_dispatch_without_passing_ci_is_refused(self):
        self.api["runs"] = []; self.assert_refused(self.check("workflow_dispatch"), "has not passed")
    def test_manual_dispatch_off_master_is_refused_and_on_master_with_ci_is_eligible(self):
        self.assert_refused(self.check("workflow_dispatch", GITHUB_REF="refs/heads/feature"), "started on master")
        self.assertEqual(self.check("workflow_dispatch").returncode, 0)
    def test_unexpected_events_are_refused(self):
        for name in ("push", "pull_request_target", "schedule", "repository_dispatch"):
            with self.subTest(event=name): self.assert_refused(self.check(name), "not from")
    def test_ci_on_another_sha_branch_event_or_workflow_does_not_count(self):
        for change in ({"head_sha": "d" * 40}, {"event": "pull_request"}, {"head_branch": "feature"}, {"conclusion": "failure"},
                       {"path": ".github/workflows/deploy.yml"}, {"head_repository": {"full_name": "attacker/opportunity-platform"}}):
            with self.subTest(change=change):
                self.api["runs"] = [self.ci_run(41, **change)]; self.assert_refused(self.check("workflow_dispatch"), "has not passed")
    def test_a_skipped_missing_or_failed_job_is_not_a_pass(self):
        for jobs in ([{"name": n, "conclusion": "skipped" if n == JOBS[2] else "success"} for n in JOBS],
                     [{"name": n, "conclusion": "success"} for n in JOBS[:-1]],
                     [{"name": n, "conclusion": "success"} for n in JOBS] + [{"name": "extra", "conclusion": "failure"}]):
            with self.subTest(jobs=[j["conclusion"] for j in jobs]):
                self.api["jobs"] = {"41": jobs}; self.assert_refused(self.check(), "every required job")
    def test_a_later_fully_green_attempt_counts(self):
        self.api["runs"] = [self.ci_run(41), self.ci_run(42)]
        self.api["jobs"] = {"41": [{"name": n, "conclusion": "success"} for n in JOBS], "42": [{"name": JOBS[0], "conclusion": "failure"}]}
        self.assertEqual(self.check().returncode, 0)
    def test_candidate_must_be_the_trigger_and_the_checkout(self):
        self.assert_refused(self.check(REVIEWED_SHA="d" * 40), "differs from the commit")
        self.assert_refused(self.check(REVIEWED_SHA=self.sha[:12]), "full 40-character")
        self.event["workflow_run"]["head_sha"] = "d" * 40; self.api["head"] = "d" * 40
        self.assert_refused(self.check(REVIEWED_SHA="d" * 40), "checkout is not the candidate")
    def test_github_api_failure_refuses(self):
        self.api["fail"] = True; self.assert_refused(self.check(), "GitHub API request failed")


class ConfigTests(unittest.TestCase):
    VALID = {"GCP_PROJECT": "fixture-project-1", "GCP_INSTANCE": "opportunity-platform", "GCP_ZONE": "us-central1-a",
             "GCP_WORKLOAD_IDENTITY_PROVIDER": "projects/123456789/locations/global/workloadIdentityPools/github/providers/opportunity-platform",
             "GCP_DEPLOY_SERVICE_ACCOUNT": "opportunity-deploy@fixture-project-1.iam.gserviceaccount.com", "REMOTE_RUN_AS": "fixture_owner"}
    def config(self, **changes):
        env = {k: v for k, v in os.environ.items() if k not in self.VALID}; env.update(self.VALID); env.update(changes)
        env = {k: v for k, v in env.items() if v is not None}
        return subprocess.run(["python3", "-I", str(BASE / "scripts/release-candidate.py"), "config"], env=env, capture_output=True, text=True)
    def test_complete_configuration_passes(self):
        self.assertEqual(self.config().returncode, 0)
    def test_missing_wif_configuration_fails_before_anything_else(self):
        for name in ("GCP_WORKLOAD_IDENTITY_PROVIDER", "GCP_DEPLOY_SERVICE_ACCOUNT", "GCP_PROJECT", "GCP_INSTANCE", "GCP_ZONE", "REMOTE_RUN_AS"):
            for value in (None, ""):
                with self.subTest(name=name, value=value):
                    r = self.config(**{name: value}); self.assertEqual(r.returncode, 1); self.assertIn(f"{name} is not set", r.stderr)
    def test_malformed_values_are_named_but_not_echoed(self):
        for name, value in (("GCP_WORKLOAD_IDENTITY_PROVIDER", "projects/x/providers/oops-value"), ("GCP_DEPLOY_SERVICE_ACCOUNT", "someone@gmail.example"),
                            ("REMOTE_RUN_AS", "root; id"), ("GCP_ZONE", "us-central1")):
            with self.subTest(name=name):
                r = self.config(**{name: value}); self.assertEqual(r.returncode, 1)
                self.assertIn(f"{name} is not well formed", r.stderr); self.assertNotIn(value, r.stdout + r.stderr)


class SummaryTests(unittest.TestCase):
    """The job summary says what happened, and is never a success production does not confirm."""
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup); self.root = pathlib.Path(self.tmp.name)
    def status(self, revision=SHA, migrations=11):
        return ("Configured image: opportunity-app:x\nRunning immutable image: sha256:" + "e" * 64 + "\nActual access mode: cloudflare\n"
                "No pending release journal\nDEPLOYED_REVISION=" + revision + "\nDEPLOYED_IMAGE=sha256:" + "e" * 64 + "\n"
                + "".join(f"MIGRATION={v}:{'0' * 96}\n" for v in range(1, migrations + 1)) + "MIGRATIONS_END\n")
    def summary(self, gate="", deploy="", status=None, **steps):
        files = {}
        for name, text in (("gate", gate), ("deploy", deploy), ("status", status)):
            if text is not None:
                files[name] = self.root / name; files[name].write_text(text)
        env = {k: v for k, v in os.environ.items() if not k.startswith("STEP_")}
        env.update(REVIEWED_SHA=SHA, **{k.upper(): v for k, v in steps.items()})
        args = ["summary"] + [f"--{'status' if n == 'status' else n + '-outcome'}={p}" for n, p in files.items()]
        return subprocess.run(["python3", "-I", str(BASE / "scripts/release-candidate.py"), *args], env=env, capture_output=True, text=True)
    ok = dict(step_config="success", step_candidate="success", candidate_result="ELIGIBLE", step_auth="success", step_gate="success")
    def test_verified_success_lists_every_fact(self):
        r = self.summary(gate=f"PREVIOUS_REVISION={'a' * 40}\nOUTCOME=DEPLOY\nRELEASE_TYPE=code-only\n",
                         deploy="OUTCOME=DEPLOYED\nCHANGED=yes\nROLLBACK_REF=opportunity-app:rollback-t\nJOURNAL=t\n",
                         status=self.status(), step_build="success", step_preflight="success", step_deploy="success", **self.ok)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        for expected in ("**Result: SUCCESS**", f"| Candidate SHA | `{SHA}` |", f"| Previously deployed SHA | `{'a' * 40}` |",
                         f"| Running SHA | `{SHA}` |", "| Database migrations | 0011 |", "| Release type | code-only |",
                         "| Image verified | yes", "| Transfer checksum verified | yes", "| Application ready | yes",
                         "| Authentication enforced | yes (ACCESS_MODE=cloudflare", "| Rollback image retained | yes (`opportunity-app:rollback-t`)"):
            self.assertIn(expected, r.stdout)
    def test_reported_success_that_production_contradicts_fails(self):
        r = self.summary(deploy="OUTCOME=DEPLOYED\n", status=self.status("a" * 40), step_build="success", step_preflight="success", step_deploy="success", **self.ok)
        self.assertEqual(r.returncode, 1); self.assertIn("**Result: FAILED**", r.stdout); self.assertNotIn("SUCCESS", r.stdout)
        r = self.summary(deploy="OUTCOME=DEPLOYED\n", status=None, step_deploy="success", **self.ok)
        self.assertEqual(r.returncode, 1); self.assertIn("**Result: FAILED**", r.stdout)
    def test_stale_already_deployed_and_disabled_runs_are_not_failures(self):
        r = self.summary(step_config="success", step_candidate="success", candidate_result="SKIPPED_STALE_REVISION", master_head="c" * 40,
                         step_auth="skipped", step_gate="skipped", step_deploy="skipped")
        self.assertEqual(r.returncode, 0); self.assertIn("**Result: SKIPPED_STALE_REVISION**", r.stdout); self.assertIn("c" * 40, r.stdout)
        self.assertIn("No deployment was attempted", r.stdout)
        r = self.summary(gate="OUTCOME=ALREADY_DEPLOYED\n", status=self.status(), step_deploy="skipped", **self.ok)
        self.assertEqual(r.returncode, 0); self.assertIn("**Result: ALREADY_DEPLOYED**", r.stdout); self.assertIn("| Production changed | no |", r.stdout)
    def test_migration_required_names_the_migrations_and_the_owner_action(self):
        r = self.summary(gate="OUTCOME=MIGRATION_REQUIRED\nPENDING_MIGRATIONS=0012_fixture.sql\n", step_build="skipped", **dict(self.ok, step_gate="failure"))
        self.assertEqual(r.returncode, 1); self.assertIn("**Result: MIGRATION_REQUIRED**", r.stdout)
        self.assertIn("`0012_fixture.sql`", r.stdout); self.assertIn("verified manual migration rollout", r.stdout)
    def test_rollback_and_unknown_outcomes_are_failures_that_say_so(self):
        r = self.summary(deploy="OUTCOME=ROLLED_BACK\nSTAGE=loading image\nROLLBACK=succeeded\n", status=self.status("a" * 40), step_deploy="failure", **self.ok)
        self.assertEqual(r.returncode, 1); self.assertIn("**Result: ROLLED_BACK**", r.stdout); self.assertIn("| Rollback | succeeded |", r.stdout)
        self.assertIn("| Failed or stopping stage | loading image |", r.stdout); self.assertIn("| Recovery required | no |", r.stdout)
        r = self.summary(step_deploy="failure", **self.ok)
        self.assertEqual(r.returncode, 1); self.assertIn("**Result: RECOVERY_REQUIRED**", r.stdout); self.assertIn("| Recovery required | yes |", r.stdout)
        self.assertIn("Do not re-run the deploy", r.stdout)
    def test_failures_before_production_name_their_stage(self):
        for steps, stage in ((dict(step_config="failure"), "configuration"), (dict(self.ok, step_auth="failure"), "GCP authentication"),
                             (dict(self.ok, step_build="failure"), "image build"), (dict(self.ok, step_build="success", step_preflight="failure"), "preflight")):
            with self.subTest(stage=stage):
                r = self.summary(**steps); self.assertEqual(r.returncode, 1); self.assertIn(stage, r.stdout); self.assertIn("| Production changed | no |", r.stdout)
        r = self.summary(step_config="success", step_candidate="failure", candidate_reason="continuous integration has not passed")
        self.assertIn("**Result: REFUSED**", r.stdout); self.assertIn("continuous integration has not passed", r.stdout)
    def test_nothing_from_the_status_beyond_its_facts_is_printed(self):
        status = self.status() + "POSTGRES_PASSWORD=" + MARKER + "\n"
        r = self.summary(deploy="OUTCOME=DEPLOYED\n", status=status, step_deploy="success", **self.ok)
        self.assertNotIn(MARKER, r.stdout + r.stderr)


class WorkflowTests(unittest.TestCase):
    """What actionlint cannot know: the deploy workflow's trust boundary and order of operations."""
    def setUp(self):
        self.deploy = (BASE / ".github/workflows/deploy.yml").read_text()
    def step(self, marker): return self.deploy.index(marker, self.deploy.index("    steps:\n"))
    def test_deployment_stays_behind_the_owner_switch_and_trusted_triggers(self):
        condition = self.deploy.split("    if: >-\n", 1)[1].split("    runs-on:", 1)[0]
        self.assertTrue(condition.strip().startswith("vars.DEPLOY_ENABLED == 'true' && ("), condition)
        for clause in ("github.event.workflow_run.conclusion == 'success'", "github.event.workflow_run.event == 'push'",
                       "github.event.workflow_run.head_branch == 'master'", "github.event.workflow_run.head_repository.full_name == github.repository",
                       "github.event.workflow_run.repository.full_name == github.repository",
                       "github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/master'"):
            self.assertIn(clause, condition)
        for path in (BASE / ".github/workflows").glob("*.yml"):
            self.assertNotRegex(path.read_text(), r"DEPLOY_ENABLED\s*(=|:)\s*['\"]?true|variable set DEPLOY_ENABLED", path.name)
    def test_permissions_are_granted_to_the_deploy_job_alone(self):
        self.assertIn("\npermissions: {}\n", self.deploy)
        self.assertIn("    permissions:\n      contents: read\n      actions: read\n      id-token: write\n", self.deploy)
        self.assertIn("\npermissions: {}\n", (BASE / ".github/workflows/backup.yml").read_text())
        self.assertNotIn("write-all", self.deploy); self.assertEqual(self.deploy.count("id-token: write"), 1)
    def test_checks_run_before_credentials_and_head_is_checked_before_build_and_before_change(self):
        order = ["actions/checkout@", "release-candidate.py config", "release-candidate.py check", "google-github-actions/auth@",
                 "deploy-vm.sh --gate", "run: ./scripts/release-image.sh", "deploy-vm.sh --preflight", 'deploy-vm.sh "dist/']
        positions = [self.step(marker) for marker in order]; self.assertEqual(positions, sorted(positions))
        self.assertIn("if: steps.candidate.outputs.outcome == 'ELIGIBLE'", self.deploy.split("google-github-actions/auth@", 1)[1].split("with:", 1)[0])
        self.assertIn("REQUIRE_CURRENT_HEAD: 'yes'", self.deploy)
        self.assertNotIn("--migration-rollout", self.deploy.split("jobs:", 1)[1])
        self.assertIn("cancel-in-progress: false", self.deploy); self.assertIn("group: production", self.deploy)
    def test_no_runner_context_where_it_is_unavailable_and_actions_are_pinned(self):
        self.assertNotIn("${{ runner.", self.deploy)
        for path in (BASE / ".github/workflows").glob("*.yml"):
            for uses in re.findall(r"uses:\s*(\S+)", path.read_text()):
                self.assertRegex(uses, r"^[\w.-]+/[\w.-]+@[0-9a-f]{40}$", f"{path.name}: {uses}")
    def test_required_jobs_are_every_ci_job(self):
        hygiene = (BASE / ".github/workflows/hygiene.yml").read_text()
        names = re.findall(r"^    name: (.+)$", hygiene, re.M)
        spec = importlib.util.spec_from_file_location("release_candidate", BASE / "scripts/release-candidate.py")
        module = importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
        self.assertEqual(sorted(module.REQUIRED_JOBS), sorted(names)); self.assertEqual(sorted(JOBS), sorted(names))


class DockerfileTests(unittest.TestCase):
    """CI never builds the image, so whatever the compiler reads from outside crates/ at build time
    has to be copied into the server stage, or every release fails at its build."""
    def test_files_compiled_into_the_binary_are_in_the_build_stage(self):
        root = BASE.resolve()
        stage = (root / "Dockerfile").read_text().split(" AS server\n", 1)[1].split("\nFROM ", 1)[0]
        copied = [src.rstrip("/") for line in re.findall(r"^COPY (.+)$", stage, re.M) for src in line.split()[:-1] if not src.startswith("--")]
        needed = set()
        for path in (root / "crates").glob("*/src/**/*.rs"):
            code = path.read_text().split("#[cfg(test)]", 1)[0]
            crate = root / "crates" / path.relative_to(root / "crates").parts[0]
            for macro, argument in re.findall(r'(include_str|include_bytes|sqlx::migrate)!\("([^"]+)"\)', code):
                target = ((crate if macro == "sqlx::migrate" else path.parent) / argument).resolve()
                needed.add(str(target.relative_to(root)))
        self.assertIn("config/sources.seed.toml", needed)
        self.assertEqual([n for n in sorted(needed) if not any(n == c or n.startswith(c + "/") for c in copied)], [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
