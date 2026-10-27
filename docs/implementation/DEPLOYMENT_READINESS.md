# Deployment readiness (2026-10-27, second pass)

Production, read from the VM: `1043ab9`, `ACCESS_MODE=cloudflare`, migrations 1-5, 20 GB free disk,
about 370 MB available memory, no swap, no backup on the VM. Nothing from this branch is deployed.

| Stage | Ready? | What it rests on / what is missing |
| --- | --- | --- |
| Locally | **Yes** | Full suite green on the final tree (TEST_RESULTS, second pass); local native-mode browser check |
| GitHub CI | **Ready to run, not verified** | Workflows parse and their steps pass locally; never run in Actions (not pushed); `actionlint` not run here |
| First migration rollout (0006-0009) | **Not yet: owner steps** | Commit, push, CI green, merge to master; Mac backup + `restore-rehearsal.sh`; then `deploy-vm.sh --migration-rollout` |
| Automated code-only CD | **Not yet** | Above, plus the WIF/IAM bootstrap and `DEPLOY_ENABLED=true` |
| Native-auth cutover (public landing) | **Not yet: owner decision** | TOTP or password-only; then the ladder in docs/AUTHENTICATION.md and a Cloudflare policy change |
| Unattended job applications | **No** | e2-micro cannot run Chromium (DEPLOYMENT.md); larger VM, `BROWSER_WORKER_ALLOW_EXTERNAL`, enabled employer policies, live provider validation |

## Go / no-go for the first production rollout

**No-go today; go once these hold, in order:**
1. The owner reviews and authorizes committing the uncommitted work; branch pushed, PR opened.
2. `continuous integration` passes in GitHub on that commit, including `acceptance` and the backup round trip.
3. Merged to `master`.
4. On the Mac: `deploy-vm.sh --fetch-backup` and `restore-rehearsal.sh` pass, under 24 hours before step 5.
5. `deploy-vm.sh --migration-rollout <backup dir> <archive>` from a clean checkout of that master commit.
6. `deploy-vm.sh --status` shows the new revision and migrations 1-9.
7. A browser check behind Cloudflare Access (unchanged in this release).

What makes it safe once those hold:
- The gate refuses unknown, diverged or in-progress production states.
- Image recovery is automatic and verified.
- The migration is covered by a rehearsed backup.
- Authentication does not change in this release.

Remaining risk: a migrated release that fails needs a manual database restore for the previous image
(documented, not automated).

## Other facts
- Runtime image: `poppler-utils` (pdftotext) present in the Dockerfile; `APP_REVISION` now exposed.
- Tunnel image pinned to the running cloudflared build; effective on the next tunnel recreate.
- Browser automation in production: still off on the e2-micro.
- Backups: pulled to the owner's Mac; continuous off-host backup is optional and off.
