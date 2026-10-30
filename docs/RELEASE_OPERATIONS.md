# Release operations

Production releases require an approved candidate, compatible SQLx migration history and verified deployment state. Code-only releases can run through GitHub Actions; schema-changing releases require an explicitly owner-authorized manual rollout with a fresh, restore-verified backup. Manual workflow dispatch does not bypass that requirement.

Run operator commands from the repository checkout unless a step names the VM. The owner performs Git writes, infrastructure configuration and production mutations. These procedures describe expected behavior, not a completed production rehearsal. Historical production observations must be checked again before use. [DEPLOYMENT.md](DEPLOYMENT.md) describes infrastructure and configuration; [AUTHENTICATION.md](AUTHENTICATION.md) owns the separately authorized Access policy cutover.

## Prerequisites

Use a clean checkout of the reviewed full commit SHA with Git, Docker Buildx, Python 3, Google Cloud CLI, GitHub CLI and GnuPG available. The restore rehearsal needs Python's tar extraction filter support and a local Docker daemon. The VM requires Docker Compose, Bash, GNU `timeout`, `flock`, `sha256sum`, at least 3 GB free on `/`, a writable `/opt/opportunity` and mode-600 `.env.prod`. Keep runtime secrets on the VM and the backup private key on the owner's machine.

Verify IAP connectivity, OS Login and the firewall rule for `35.235.240.0/20` to TCP 22. Docker access must work directly or through passwordless sudo. `REMOTE_RUN_AS` lets the CI login execute commands as the owner of `/opt/opportunity`; validate that user's passwordless sudo access before enabling deployment. Do not build on the e2-micro or use `migrate-to-cloud.sh` to upgrade existing production data.

## Release gates

| Step | Refuses when | Exit / outcome |
| --- | --- | --- |
| Job `if` | `DEPLOY_ENABLED` is not `true`; the trigger is not a successful push CI run on `master` of this repository, or a manual run on `master` | job skipped, nothing requested |
| `release-candidate.py config` | a deploy variable is missing or malformed (named, never echoed) | 1 |
| `release-candidate.py check` | pull-request, fork, other-branch, other-workflow or unsuccessful CI; a manual run without a fully green push CI run on that SHA; candidate, trigger and checkout SHAs do not all match; abbreviated SHA; GitHub API failure | 1 `REFUSED` |
| | the candidate is no longer `master` HEAD | 11 `SKIPPED_STALE_REVISION`, no credentials requested |
| WIF auth | only reached when the candidate is `ELIGIBLE` | |
| `deploy-vm.sh --gate` | Unresolved state, divergence, out-of-band changes, failed or incompatible migrations, candidate outside `origin/master`, or stale HEAD when enforced | 1 refused; 2 unresolved; 11 stale |
| | production already runs the candidate | 10 `ALREADY_DEPLOYED`, no build |
| | the candidate adds migrations | 12 `MIGRATION_REQUIRED`, no build |
| `release-image.sh` | checkout dirty (ignored files excepted), SHA not the full HEAD, a tracked credential, image carries a secret-shaped file | 1, nothing packaged |
| `deploy-vm.sh --preflight` | Unreachable VM, unavailable tools, wrong architecture, insufficient disk, invalid environment or missing archive/checksum | Nonzero; no release mutation |
| `deploy-vm.sh <archive>` | Rechecks the gate before journaled transfer, load, configuration and verification; failed mutation attempts recovery when remote completion is known | 0 deployed or already running; 1 refused or recovered; 2 recovery required; 11 stale; 12 migration required |
| `--status` + summary | the summary reports SUCCESS only when `--status` shows the candidate running | |

`release-candidate.py` requires the exact full SHA and every required CI job to succeed. The deployment workflow sets `REQUIRE_CURRENT_HEAD=yes`, which rechecks master HEAD at both gate calls. Manual `deploy-vm.sh` invocations do not enforce current remote HEAD unless that variable is set; their default trusted reference is the local `origin/master`. Refresh and review that reference before a manual release.

The migration gate compares file-byte SHA-384 checksums with `_sqlx_migrations`. Missing or edited applied migrations, failed migrations, out-of-order additions, divergent revisions and configured/running image mismatches are refusal conditions. Exit 10 means the revision is already deployed; it is not a fresh image-health verification. Review status and application behavior separately.

`--preflight` inspects prerequisites and reports a failure count. It does not fully render Compose, validate archive contents or prove sufficient memory under load. `--dry-run` checks local artifact identity and prints the intended action but does not contact production or evaluate its migration gate. `--check-transfer` writes and removes an empty probe; obtain authorization before using it.

## Immutable images

Build from a clean reviewed checkout. Ignored files are excluded, but any untracked nonignored file prevents the build. Set `REVIEWED_SHA` to the full HEAD; `VERSION`, if set, must equal it. Production builds accept only `PLATFORM=linux/amd64` and `TARGET=runtime`.

```bash
REVIEWED_SHA="$(git rev-parse HEAD)" ./scripts/release-image.sh
```

The builder uses `git archive` for its build context, verifies architecture and revision labels, runs `image-check.sh`, and packages `dist/opportunity-app-<full-sha>-amd64.tar.gz` with a `.sha256` file. `release-artifact.py` checks archive structure, checksum, revision and image identity before deployment. Existing tags or archives are refused to prevent replacement of a reviewed artifact. Preserve the reviewed artifact and checksum together; do not rename a different image to satisfy the gate.

The VM helper retains the previous image as `opportunity-app:rollback-<release-token>` and records configuration under `.release-state`, later moved to `.release-history/<release-token>`. Keep these until recovery is no longer needed. The archive is removed from the VM after a verified load, but retained images still consume disk.

## 1. One-time owner configuration

The following values match the deployment script defaults. Confirm them against the intended environment. Read project and repository IDs rather than inferring them from names, and replace `VM_OWNER_USER` with the user reported by the VM.

```bash
PROJECT=project-484f9dc3-1a36-4b55-9e8
REPO=alpblkba/opportunity-platform
INSTANCE=opportunity-platform
ZONE=us-central1-a
SA="opportunity-deploy@${PROJECT}.iam.gserviceaccount.com"
NUMBER="$(gcloud projects describe "$PROJECT" --format='value(projectNumber)')"
REPO_ID="$(gh api "repos/$REPO" --jq .id)"            # 1432571935 on 2026-10-29
OWNER_ID="$(gh api "repos/$REPO" --jq .owner.id)"     # 67732998 on 2026-10-29
# The Linux user that owns /opt/opportunity on the VM (read-only):
gcloud compute ssh "$INSTANCE" --zone "$ZONE" --project "$PROJECT" --tunnel-through-iap --command 'stat -c %U /opt/opportunity'
VM_USER="<VM_OWNER_USER>"
```

### 1.1 Workload Identity Federation

Create the pool, provider and service account only after checking that they do not already exist. For an existing resource, inspect its configuration before applying an owner-approved change. The proposed provider condition restricts tokens to this repository and owner by numeric ID, the `production` environment, `master`, and `deploy.yml` or `backup.yml`. These commands configure federation without creating a service-account key; section 1.3 checks for existing user-managed keys.

```bash
gcloud iam workload-identity-pools create github --project "$PROJECT" --location global \
  --display-name "GitHub Actions"
gcloud iam workload-identity-pools providers create-oidc opportunity-platform --project "$PROJECT" \
  --location global --workload-identity-pool github \
  --issuer-uri https://token.actions.githubusercontent.com \
  --attribute-mapping "google.subject=assertion.sub,attribute.repository_id=assertion.repository_id,attribute.environment=assertion.environment,attribute.ref=assertion.ref,attribute.workflow_ref=assertion.workflow_ref" \
  --attribute-condition "assertion.repository_id == '${REPO_ID}' && assertion.repository_owner_id == '${OWNER_ID}' && assertion.environment == 'production' && assertion.ref == 'refs/heads/master' && (assertion.workflow_ref == '${REPO}/.github/workflows/deploy.yml@refs/heads/master' || assertion.workflow_ref == '${REPO}/.github/workflows/backup.yml@refs/heads/master')"
gcloud iam service-accounts create opportunity-deploy --project "$PROJECT" \
  --display-name "GitHub deploy (federated, no keys)"
gcloud iam service-accounts add-iam-policy-binding "$SA" --project "$PROJECT" \
  --role roles/iam.workloadIdentityUser \
  --member "principalSet://iam.googleapis.com/projects/${NUMBER}/locations/global/workloadIdentityPools/github/attribute.repository_id/${REPO_ID}"
```

### 1.2 IAM, IAP and OS Login

| Role | Where | Why |
| --- | --- | --- |
| `roles/compute.osAdminLogin` | this instance only | OS Login as `sa_<id>` with sudo, which `REMOTE_RUN_AS` needs to act as the owning user |
| `roles/iap.tunnelResourceAccessor` | project, condition port 22 | the IAP tunnel `gcloud compute ssh --tunnel-through-iap` uses |
| `roles/compute.viewer` | project | `gcloud compute ssh` reads the instance and project metadata (read-only) |
| `roles/iam.serviceAccountUser` | the VM's own service account, only if it has one | required by OS Login for instances that run as a service account |

```bash
gcloud compute instances add-iam-policy-binding "$INSTANCE" --project "$PROJECT" --zone "$ZONE" \
  --member "serviceAccount:$SA" --role roles/compute.osAdminLogin
gcloud projects add-iam-policy-binding "$PROJECT" --member "serviceAccount:$SA" \
  --role roles/iap.tunnelResourceAccessor --condition 'expression=destination.port == 22,title=iap-ssh-only'
gcloud projects add-iam-policy-binding "$PROJECT" --member "serviceAccount:$SA" \
  --role roles/compute.viewer --condition None
# Only if this prints an address:
VM_SA="$(gcloud compute instances describe "$INSTANCE" --zone "$ZONE" --project "$PROJECT" --format='value(serviceAccounts[].email)')"
[[ -z "$VM_SA" ]] || gcloud iam service-accounts add-iam-policy-binding "$VM_SA" --project "$PROJECT" \
  --member "serviceAccount:$SA" --role roles/iam.serviceAccountUser
```

OS Login must already be on (`enable-oslogin=TRUE` on the instance or the project); without it `gcloud compute ssh` writes keys into instance metadata. Check before changing anything:

```bash
gcloud compute instances describe "$INSTANCE" --zone "$ZONE" --project "$PROJECT" --format='value(metadata.items)'
gcloud compute project-info describe --project "$PROJECT" --format='value(commonInstanceMetadata.items)'
gcloud compute firewall-rules list --project "$PROJECT" --filter='sourceRanges:35.235.240.0/20' --format='table(name,allowed,targetTags)'
```

`osAdminLogin` is root on the VM. Whoever can push to `master` can therefore, through CI, run code on the VM; the bootstrap reviewer below and branch protection on `master` are what limit that.

### 1.3 Identity verification

```bash
gcloud iam workload-identity-pools providers describe opportunity-platform --workload-identity-pool github \
  --location global --project "$PROJECT" --format='value(state,attributeCondition)'   # ACTIVE and the condition above
gcloud iam service-accounts keys list --iam-account "$SA" --project "$PROJECT" --managed-by user   # must print nothing
gcloud iam service-accounts get-iam-policy "$SA" --project "$PROJECT"                       # only the principalSet above
gcloud projects get-iam-policy "$PROJECT" --flatten bindings \
  --filter "bindings.members:serviceAccount:$SA" --format 'table(bindings.role,bindings.condition.expression)'
gcloud compute instances get-iam-policy "$INSTANCE" --zone "$ZONE" --project "$PROJECT"
```

### 1.4 GitHub `production` environment

Create it before setting any variable: a job that references a missing environment creates it unprotected. Bootstrap: only `master` may deploy, and the owner approves each run.

```bash
gh api -X PUT "repos/$REPO/environments/production" --input - <<JSON
{"wait_timer": 0, "prevent_self_review": false, "reviewers": [{"type": "User", "id": ${OWNER_ID}}],
 "deployment_branch_policy": {"protected_branches": false, "custom_branch_policies": true}}
JSON
gh api -X POST "repos/$REPO/environments/production/deployment-branch-policies" -f name=master -f type=branch
# Read-only:
gh api "repos/$REPO/environments/production" --jq '{protection_rules, deployment_branch_policy}'
gh api "repos/$REPO/environments/production/deployment-branch-policies" --jq '[.branch_policies[].name]'
```

After section 4 passes, the owner may remove the required reviewer while retaining the branch policy. Code-only releases then proceed without per-run approval. Retaining the reviewer is also supported; a run approved after a newer merge exits as `SKIPPED_STALE_REVISION`.

```bash
gh api -X PUT "repos/$REPO/environments/production" --input - <<'JSON'
{"wait_timer": 0, "reviewers": [], "deployment_branch_policy": {"protected_branches": false, "custom_branch_policies": true}}
JSON
```

### 1.5 Repository variables

`DEPLOY_ENABLED` is not in this list; it is set in section 4, last.

```bash
gh variable set GCP_PROJECT --repo "$REPO" --body "$PROJECT"
gh variable set GCP_INSTANCE --repo "$REPO" --body "$INSTANCE"
gh variable set GCP_ZONE --repo "$REPO" --body "$ZONE"
gh variable set GCP_DEPLOY_SERVICE_ACCOUNT --repo "$REPO" --body "$SA"
gh variable set GCP_WORKLOAD_IDENTITY_PROVIDER --repo "$REPO" \
  --body "projects/${NUMBER}/locations/global/workloadIdentityPools/github/providers/opportunity-platform"
gh variable set REMOTE_RUN_AS --repo "$REPO" --body "$VM_USER"
# Read-only: the same shape check the job runs first (names a bad value, never prints it)
GCP_PROJECT="$PROJECT" GCP_INSTANCE="$INSTANCE" GCP_ZONE="$ZONE" GCP_DEPLOY_SERVICE_ACCOUNT="$SA" \
  GCP_WORKLOAD_IDENTITY_PROVIDER="projects/${NUMBER}/locations/global/workloadIdentityPools/github/providers/opportunity-platform" \
  REMOTE_RUN_AS="$VM_USER" python3 -I scripts/release-candidate.py config
gh variable list --repo "$REPO"
```

### 1.6 Disabling and revoking access

The first two controls prevent future deployment jobs from starting but do not cancel an active job. Disabling the federation provider prevents new exchanges; already issued credentials may remain valid until expiry. Revoke VM access or disable the service account when immediate credential revocation is required. The provider and service account are shared with optional backups, so those changes also affect backup access.

```bash
gh variable set DEPLOY_ENABLED --repo "$REPO" --body false   # jobs skip; nothing is requested
gh workflow disable deploy --repo "$REPO"
gcloud iam workload-identity-pools providers disable opportunity-platform --workload-identity-pool github \
  --location global --project "$PROJECT"                       # prevents new exchanges; existing credentials may remain valid until expiry
gcloud compute instances remove-iam-policy-binding "$INSTANCE" --project "$PROJECT" --zone "$ZONE" \
  --member "serviceAccount:$SA" --role roles/compute.osAdminLogin
gcloud iam service-accounts disable "$SA" --project "$PROJECT"
```

Do not cancel a run during deployment without accepting an unresolved result. Let it finish when possible; after interruption, run `./scripts/deploy-vm.sh --status` and follow the recovery procedure if a journal remains. If optional backups must also stop, set `BACKUP_ENABLED=false` and disable the `backup` workflow. GitHub failure notifications depend on the owner's notification settings; the workflows do not configure email delivery.

## 2. First migration rollout (manual, from the owner's machine)

The historical observation from 2026-10-27 reported revision `1043ab9` with migrations 1 through 5. The candidate schema in this checkout contains migrations `0001` through `0011`; stage 1 must read production again. If its state differs, stop and review the actual migration delta rather than relying on this baseline.

Use a reviewed commit that includes the release scripts and Dockerfile requirements. The Docker build must copy `config/sources.seed.toml`, which `product.rs` includes at compile time. The VM helper must normalize ordinary nonzero `vm-check.sh` failure counts before deciding whether to recover; a checker exit of 2 must not be mistaken for unresolved transport. An earlier CI result or image does not establish either property for a new candidate.

### Stage 1: candidate and production checks

The owner runs `git fetch origin` to update local Git references and prepares the reviewed checkout. The remaining commands inspect CI and production without replacing the running release.

```bash
git fetch origin
M="$(git rev-parse origin/master)"
# Owner: check out exactly $M with nothing untracked, e.g. a fresh worktree; `git status --short` prints nothing.
gh run list --repo "$REPO" --workflow hygiene.yml --commit "$M" --event push --json conclusion,status
./scripts/deploy-vm.sh --status
REVIEWED_SHA="$M" ./scripts/deploy-vm.sh --gate; echo "gate exit $?"
```

For the historical baseline, expect successful push CI for `$M`, a full deployed revision beginning `1043ab9`, separate `MIGRATION=<version>:<checksum>` records for versions 1 through 5, `MIGRATIONS_END`, and no `MIGRATION_FAILED` or unresolved journal. The gate should exit 12 and name `0006_cv_journey.sql` through `0011_legacy_notification_cutover.sql`. Stop on any different result until you understand the actual state. The `gh run list` command is a preliminary summary; inspect the successful run and confirm every required CI job passed, including jobs named in `release-candidate.py`.

### Stage 2: recovery point

Run backup and rehearsal on the owner's machine with the public and private backup keys available. Replace angle-bracket placeholders before executing commands.

```bash
BACKUP_OUT_DIR=~/Backups/opportunity BACKUP_ENCRYPT_TO=<FINGERPRINT> ./scripts/deploy-vm.sh --fetch-backup
B=~/Backups/opportunity/opportunity-<stamp>          # the directory it printed
./scripts/restore-rehearsal.sh "$B"                   # writes $B/restore-verified.json
```

Review the manifest and successful rehearsal result using [backups and restore rehearsals](#backups-and-restore-rehearsals). Stage 5 must start within 24 hours of backup creation, not merely within 24 hours of the rehearsal. Production revision and migration checksums must still match the backup.

### Stage 3: build

```bash
REVIEWED_SHA="$M" ./scripts/release-image.sh          # linux/amd64; slow under emulation on Apple silicon
ls -l "dist/opportunity-app-${M}-amd64.tar.gz"
```

### Stage 4: preflight, read-only

```bash
./scripts/deploy-vm.sh --preflight "dist/opportunity-app-${M}-amd64.tar.gz"
REVIEWED_SHA="$M" ./scripts/deploy-vm.sh --gate --migration-rollout "$B"; echo "gate exit $?"   # expect 0
REVIEWED_SHA="$M" ./scripts/deploy-vm.sh --dry-run --migration-rollout "$B" "dist/opportunity-app-${M}-amd64.tar.gz"
```

The gate must print `DEPLOY: migration rollout of ... with a verified recovery point`.

### Stage 5: rollout

Obtain explicit owner authorization for the schema change after reviewing the recovery point, migration list, preflight and recovery limitations. The manifest proves checks performed by the scripts; it is not a record of owner authorization. Keep `ACCESS_MODE=cloudflare` and the Access policy unchanged throughout this rollout.

```bash
REVIEWED_SHA="$M" ./scripts/deploy-vm.sh --migration-rollout "$B" "dist/opportunity-app-${M}-amd64.tar.gz"; echo "deploy exit $?"
```

### Stage 6: verification

```bash
./scripts/deploy-vm.sh --status
REVIEWED_SHA="$M" ./scripts/deploy-vm.sh --gate; echo "gate exit $?"   # expect 10 ALREADY_DEPLOYED
```

For this candidate, expect `DEPLOYED_REVISION=$M`, separate migration records for versions 1 through 11, `MIGRATIONS_END`, `Actual access mode: cloudflare`, and no failed migration or pending journal. Sign in through Cloudflare Access and verify authenticated dashboard data. Migration `0011_legacy_notification_cutover.sql` suppresses untouched pending notifications created before migration `0006`; seen rows, attempted sends and provider receipts are preserved.

### If stage 5 does not exit 0

| Exit | Meaning | Do |
| --- | --- | --- |
| 1, gate message | refused before any change | nothing changed; fix the cause, repeat from the stage it names |
| 1, after a change | failed and the previous image was restored and verified; migrations had not run | `--status` must show `1043ab9` and migrations 1-5 |
| 2 | recovery required: typically the new image migrated, then failed, and the previous image will not start against the newer schema | below |

For exit 2, follow [rollback and recovery](#rollback-and-recovery). Do not rerun deployment, remove the release journal or edit `_sqlx_migrations` to force startup. If migrations ran before failure, the previous image may require restoration into a new database.

## 3. Gate before enabling automated deployment

All of these, checked by the owner, before `DEPLOY_ENABLED=true`:

1. The reviewed release tooling is committed on `master`, and `continuous integration` passed every required job on that exact push SHA.
2. Section 2 finished: `REVIEWED_SHA="$(git rev-parse origin/master)" ./scripts/deploy-vm.sh --gate` exits 10.
3. Sections 1.1-1.3 done and verified; `keys list --managed-by user` prints nothing.
4. The `production` environment exists with the `master` branch policy and the owner as required reviewer (1.4).
5. Every variable in 1.5 is set and the local `config` check passes; `REMOTE_RUN_AS` is the owner of `/opt/opportunity`.
6. `master` is protected (pull requests required, no force pushes), since a merge to `master` becomes a deploy.
7. The owner can watch the first rehearsal.

## 4. First automated rehearsal

R1 verifies identity and transport without changing the running release. Start with current master HEAD already deployed by section 2:

```bash
gh variable set DEPLOY_ENABLED --repo "$REPO" --body true
gh workflow run deploy --repo "$REPO" --ref master
gh run watch --repo "$REPO" "$(gh run list --repo "$REPO" --workflow deploy.yml --limit 1 --json databaseId --jq '.[0].databaseId')"
```

Approve the `production` deployment when asked. Expect: candidate `ELIGIBLE`, authentication success, gate `ALREADY_DEPLOYED`, build, preflight and deploy skipped, the status step reading the VM, and the summary `ALREADY_DEPLOYED` with "Production changed: no". A successful result verifies WIF, IAP, OS Login, `sudo -u $VM_USER` and status access for that run. The VM helper may create or open its lock file; it does not replace the release. A failure at authentication points at 1.1 (condition, binding); at the gate's `gcloud compute ssh`, at 1.2 or OS Login.

R2 exercises a code-only release. Merge a small code-only change through a pull request. After CI passes on the push, the run starts by itself; approve it. Expect summary `SUCCESS` with the new SHA running, migrations unchanged, image and transfer checksum verified, authentication enforced and a rollback image retained. Confirm from the Mac with `./scripts/deploy-vm.sh --status`.

R3 checks ordering, optionally. Merge two changes in quick succession. An older run that has not passed candidate verification should end as `SKIPPED_STALE_REVISION` before requesting credentials. If it already authenticated, the later gate can still reject it as stale; do not assume every stale run avoided credentials. Confirm the newer run's actual result.

R4 checks the next schema-changing release. Its run ends `MIGRATION_REQUIRED` naming the migration and changes nothing; ship it with section 2's stages.

After R1 and R2 pass, optionally move the environment to steady state (1.4).

## Controls and regression coverage

| Condition | Control | Regression or operator check |
| --- | --- | --- |
| A pull request or fork reaches credentials | job `if` requires push CI on `master` of this repo; `check` re-verifies against the API; WIF accepts only `production`/`master`/these workflows | `CandidateTests` pull request, fork, other repository |
| CI of another commit, branch or workflow stands in | the run must be `hygiene.yml`, push, `master`, the exact SHA, every required job `success` | `test_ci_on_another_sha_branch_event_or_workflow_does_not_count`, `test_a_skipped_missing_or_failed_job_is_not_a_pass`, `test_required_jobs_are_every_ci_job` |
| An older commit overwrites a newer one | master HEAD checked by `check`, by the gate step and inside the deploy right before the VM lock | `test_an_older_master_commit_is_stale_and_changes_nothing`, `test_a_newer_master_head_stops_the_release_before_any_change` |
| A schema change ships without a backup | the gate exits 12 for any new migration unless a verified, fresh, matching rollout manifest is given; the workflow never passes one | `test_new_migration_is_never_automatic`, `test_schema_change_is_refused_before_any_mutation`, `test_checks_run_before_credentials_and_head_is_checked_before_build_and_before_change` |
| The OIDC credential file enters the image | ignored by Git and `.dockerignore`; the build context is `git archive` of the reviewed commit; `image-check.sh` fails an image carrying one | `ReleaseImageTests`, `ContainerContentTests` |
| Long-lived keys | Workflow uses WIF; inspect user-managed keys rather than assuming none exist | Section 1.3 |
| Script injection | no event field is interpolated into a `run:` script; values pass through `env` | `WorkflowTests`, actionlint |
| Token scope | top-level `permissions: {}`; the job has `contents: read`, `actions: read`, `id-token: write` | `test_permissions_are_granted_to_the_deploy_job_alone` |
| Mutable action tags | every `uses:` pinned to a full commit SHA | `test_no_runner_context_where_it_is_unavailable_and_actions_are_pinned` |
| Secrets in logs or summaries | config values named, not echoed; summary reads only listed facts | `test_malformed_values_are_named_but_not_echoed`, `test_nothing_from_the_status_beyond_its_facts_is_printed` |
| Accidental activation | `DEPLOY_ENABLED` is set by no workflow, script or test | `test_deployment_stays_behind_the_owner_switch_and_trusted_triggers` |
| Concurrent releases | workflow concurrency group plus the VM journal lock | `test_concurrent_release_holding_the_vm_lock_is_refused`, `test_signal_and_concurrent_release_leave_reconcilable_state` |

A compromised trusted branch or workflow can execute code with administrative VM privileges. Branch protection, environment review and pinned actions limit that exposure but do not remove it. `compute.viewer` grants project-wide read access, and the GitHub-hosted runner receives short-lived credentials.

The regression references in these tables are defined in [test_release.py](../scripts/test_release.py); they do not establish a current local, CI or production pass. The workflow also configures actionlint and the tests in [test_ci.py](../scripts/test_ci.py). Inspect actual run results before enabling deployment.

### Failure scenarios

| Simulated scenario | Expected result | Test |
| --- | --- | --- |
| Corrupted transfer | checksum fails on the VM, rolled back, exit 1 | `test_corrupted_transfer_is_caught_on_the_vm_and_rolled_back` |
| Interrupted transfer | rolled back, exit 1 | `test_interrupted_transfer_is_rolled_back` |
| Disk full during load | rolled back, exit 1 | `test_failed_load_such_as_a_full_disk_is_rolled_back` |
| New release never ready | rolled back, exit 1 | `test_new_release_that_never_becomes_ready_is_rolled_back` |
| Health check fails (real `vm-check.sh`) | rolled back, exit 1 | `test_real_checker_rolls_back_a_release_whose_health_fails` |
| Private data served anonymously | rolled back, exit 1 | `test_real_checker_rolls_back_a_release_that_serves_private_data_anonymously` |
| Rollback cannot be proven | exit 2 `RECOVERY_REQUIRED` | `test_unprovable_recovery_reports_recovery_required` |
| SSH/IAP lost mid-release | exit 2, journal kept for `--reconcile` | `test_lost_transport_reports_recovery_required_without_rollback`, `test_lost_transport_requires_explicit_reconciliation` |
| Lock held by another release | refused, nothing changed | `test_concurrent_release_holding_the_vm_lock_is_refused` |
| SIGTERM / SIGKILL mid-release | durable journal, explicit reconcile | `test_signal_and_concurrent_release_leave_reconcilable_state`, `test_sigkill_leaves_durable_state_for_explicit_reconciliation` |
| Previous image missing | unresolved, exit 2 | `test_missing_retained_previous_image_is_unresolved` |
| Operator edited configuration | not overwritten, needs review | `test_operator_edit_is_not_overwritten_by_reconciliation` |
| Wrong running identity or mode | not success | `test_wrong_running_immutable_identity_is_rejected`, `test_wrong_running_mode_is_not_success` |
| Failure before any change | reported as unchanged, exit 1 | `test_failure_before_any_change_is_reported_as_such` |
| Wrong run-as user value | refused before contact | `test_unsafe_run_as_user_is_refused_before_contact` |

## Backups and restore rehearsals

The documented backup policy pulls encrypted copies to the owner's Mac. Run backup and rehearsal before every production deployment; schema-changing deployments enforce the recovery-point checks. Code-only automation does not create a backup automatically, so maintain a separate backup schedule if enabling unattended releases.

Use the full 40-hex fingerprint of an encryption-capable, unexpired, unrevoked OpenPGP key. The receiving machine needs its public key for backup and private key for rehearsal; do not transfer the private key to the VM.

```bash
gpg --fingerprint you@example.com
BACKUP_OUT_DIR=~/Backups/opportunity BACKUP_ENCRYPT_TO=<FINGERPRINT> ./scripts/deploy-vm.sh --fetch-backup
./scripts/restore-rehearsal.sh ~/Backups/opportunity/opportunity-<stamp>
```

`--fetch-backup` streams `pg_dump` and the private tree over IAP, encrypting on the receiver before writing payloads to disk. It checks stream type and plaintext/ciphertext SHA-256 values, then writes a manifest containing the production revision and migration checksums. A mode-700 `.incomplete-*` directory becomes `opportunity-<stamp>/` only on completion. Failure removes the incomplete directory and leaves earlier backups intact. If production status is unavailable, a backup may still be created, but it cannot serve as a migration recovery point.

The dump precedes the file archive. Uploads add files, so the later archive should contain files referenced by the dump; the rehearsal verifies this rather than assuming it. Other concurrent writes mean this is not an atomic application-wide snapshot. Quiescing activity and choosing a recovery point remain operator responsibilities for an actual restore.

`restore-rehearsal.sh` verifies ciphertext checksums, decrypts into a temporary directory, restores into a disposable `postgres:16-alpine` container with `--network none`, and extracts files into a temporary tree. It checks migration history and every referenced document's content hash, then writes `restore-verified.json` beside the backup. It removes a previous attestation before starting; a failed rehearsal cannot leave an old success marker. Plaintext exists temporarily on the rehearsal machine and is removed during cleanup.

Before authorizing migrations, confirm the manifest matches production's current revision and migration checksums, the backup is under 24 hours old, and the attestation reports `ok=true`, `documents_missing=0` and the correct manifest hash. The release gate repeats these checks. The rehearsal does not test a production database switch or the previous application image against the restored database.

`scripts/backup.sh` creates host-local database and private-tree copies. It writes plaintext first and encrypts only when configured; `BACKUP_REQUIRE_ENCRYPTION=yes` refuses a missing recipient. It does not produce the same recovery manifest as `--fetch-backup`. A copy on the database's disk is not off-host recovery protection.

### Optional automated backups

[backup.yml](../.github/workflows/backup.yml) runs at 03:17 UTC daily or on demand only when `BACKUP_ENABLED=true`. It uses the `production` environment and the same federated identity as deployment. Configure it only after owner approval, identity verification and a tested manual backup/rehearsal. These steps create billable storage and alter GitHub configuration.

The workflow receives only the owner's public key, streams encrypted backups and uploads the completed directory to a private bucket. It does not run a restore rehearsal. Set the repository variables after creating the bucket and reviewing retention:

```bash
gcloud storage buckets create gs://REPLACE_WITH_BUCKET --project=project-484f9dc3-1a36-4b55-9e8 \
  --location=us-central1 --uniform-bucket-level-access --public-access-prevention
# Delete objects after 90 days; uploads use --no-clobber.
printf '{"rule":[{"action":{"type":"Delete"},"condition":{"age":90}}]}' > lifecycle.json
gcloud storage buckets update gs://REPLACE_WITH_BUCKET --lifecycle-file=lifecycle.json
gcloud storage buckets add-iam-policy-binding gs://REPLACE_WITH_BUCKET --role=roles/storage.objectCreator \
  --member="serviceAccount:opportunity-deploy@project-484f9dc3-1a36-4b55-9e8.iam.gserviceaccount.com"
gh variable set BACKUP_BUCKET --body REPLACE_WITH_BUCKET
gh variable set BACKUP_ENCRYPT_TO --body "<your key's full 40-hex fingerprint>"
gpg --armor --export "<your key's fingerprint>" | gh variable set BACKUP_GPG_PUBLIC_KEY
gh variable set BACKUP_ENABLED --body true
```

Run the `gh variable` commands from this repository or supply `--repo "$REPO"`. Verify the bucket is private, its lifecycle is correct and a workflow upload completes. `roles/storage.objectCreator` does not grant reading or deleting backup objects; the owner needs separate access to download them. Download a complete `opportunity-<stamp>/` directory and run `restore-rehearsal.sh` on the owner's machine. Check GitHub notification preferences for failed-run alerts; no workflow step sends failure email itself.

## Rollback and recovery

Read status before taking recovery action:

```bash
./scripts/deploy-vm.sh --status
```

A resolved status reports the configured image, running immutable image, actual access mode, full `DEPLOYED_REVISION`, `DEPLOYED_IMAGE`, `CONFIGURED_IMAGE_ID`, SQLx migration records and `MIGRATIONS_END`. An unresolved journal returns exit 2 and may prevent migration history from being reported. Preserve the journal and the last known backup metadata in that case.

For an unresolved release, run reconciliation only with owner authorization because it can accept the candidate or recreate the previous application:

```bash
./scripts/deploy-vm.sh --reconcile
```

Reconciliation accepts the candidate only after identity, configuration, mode and health checks pass. Otherwise it attempts the recorded previous image and configuration. External configuration edits, missing retained images and uncertain transport leave recovery unresolved. Do not delete `.release-state`, overwrite its records or rerun deployment to bypass this condition.

A manual image rollback is appropriate only when no unresolved journal remains and the target image is compatible with the current database. Use the retained reference recorded for the release:

```bash
./scripts/deploy-vm.sh --rollback opportunity-app:<previous tag>
./scripts/deploy-vm.sh --status
```

Manual rollback preserves current authentication settings; journal recovery restores recorded settings. Neither changes Cloudflare policy or restores data. Exit 1 after mutation means recovery passed the checks; exit 2 means it could not be established. For a refusal before mutation, inspect the stated cause rather than assuming a rollback occurred.

### Schema-changing recovery

An older binary can reject a database containing migrations absent from that binary, including `migration 5 was previously applied but is missing in the resolved migrations` for a pre-`0005` image. Never edit `_sqlx_migrations` or migration files to conceal the incompatibility. Keep Access enforced, preserve the migrated database and obtain explicit owner authorization before restoring data.

Restore the verified pre-release database into a new database, retaining the failed database for investigation. `restore.sh` accepts a plain custom-format dump or a `.gpg` dump when the executing machine has the private key. Decrypt on the owner's machine and transfer the dump through the authorized private transport; keep plaintext mode 600 and remove it after verification. On the VM, with Docker access and the script available in the deployment directory, the database restore command is:

```bash
# On the VM, from /opt/opportunity; replace the path with the protected transferred dump.
scripts/restore.sh /path/to/database.dump opportunity_restored
```

The script uses `COMPOSE_FILE` (default `docker-compose.prod.yml`), `ENV_FILE` (default `.env.prod`) and `POSTGRES_USER`. It refuses an existing target unless `ALLOW_OVERWRITE=yes`; use a new name rather than enabling overwrite for recovery. It restores only the database. Restore the corresponding private files into an isolated volume or tree and verify document hashes before using them.

The current Compose configuration derives `DATABASE_URL` from `POSTGRES_DB`; setting an unrelated `DATABASE_URL` in `.env.prod` does not redirect the application. An approved recovery configuration must select the restored database for both the application and database-status commands, preserve the previous image, and reconcile the existing release journal without overwriting evidence. Merely changing `POSTGRES_DB` and invoking `--rollback` while `.release-state` exists will not complete recovery: the journal rejects concurrent or external changes.

No complete production database-switch and journal-recovery procedure is implemented or rehearsed in these scripts. Do not treat the isolated restore command as an end-to-end recovery procedure. Prepare and rehearse the exact configuration, private-volume switch, prior-image startup and journal resolution on an isolated copy before authorizing a schema-changing production release. This remains a release-readiness constraint.
