> **Historical record.** The first-pass review export of 2026-10-27, written before the second pass and
> before the history was reconstructed. Commit hashes and "uncommitted" states in it refer to the old
> history, kept on `backup/modernization-before-rewrite-20261027`. Current state: `TASK_STATE.md`,
> `TEST_RESULTS.md` and `DEPLOYMENT_READINESS.md` beside this folder.

# Opportunity Platform: modernization session review report

| | |
| --- | --- |
| Generated | 2026-10-27, from one Claude Code session (Claude Opus 5.5) |
| Repository | `alpblkba/opportunity-platform` (private single-owner app) |
| Branch | `modernization/production-readiness`, local only, **not pushed** |
| Base | `master` == `origin/master` == `1043ab9` (untouched) |
| HEAD | `053693d`, plus an **uncommitted** working tree (owner asked: "do not commit or push anything") |
| Production | unchanged; still runs `1043ab9` (verified read-only, section 7) |

This report is written for other reviewers, human or LLM. It is self-contained: the narrative is
in sections 1–14, and the full diffs and verbatim docs are in the appendices. The owner's pre-existing
work-in-progress snapshot (`06fb29a`) is in the separate file `wip-snapshot.patch` next to this
report, because it was not written in this session.

---

## 0. Review guide

**What to review**

1. Session commits `7f00fc2..053693d` (Appendix B, about 2,200 diff lines).
2. The uncommitted working tree (Appendix C, about 420 lines including two new files).
3. Optionally, `06fb29a` (`wip-snapshot.patch`, about 7,600 lines). This is the owner's earlier uncommitted
   work, committed unchanged as a snapshot so every later change is a reviewable diff.

**What is out of scope:** Cloudflare dashboard configuration, GCP IAM state, the VM's `.env.prod`
(never read), and real personal data (never used in tests).

**Where scrutiny is most useful:** see section 12. In short:
- the deploy workflow's schema gate
- the backup pipe's failure handling
- the submission brakes' concurrency argument
- the ranking weights
- the source-alert dedupe key
- whether the auth cutover plan is safe

**Status vocabulary** (from `TASK_STATE.md`):
- `TESTED` = passes locally.
- `VERIFIED_COMPLETE` = also has CI or production evidence.

Nothing on this branch has run in GitHub Actions.

---

## 1. Project and goal

**Stack:**
- Rust workspace (`domain`, `adapters`, `core`, `api`, `cli`) on Axum and sqlx.
- PostgreSQL 16, with forward-only SQLx migrations.
- React 19, Vite and Vitest.
- A Playwright browser worker spawned per task, with a guard that only allows loopback navigation unless
  `BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised` is set.
- Docker Compose, with prebuilt images.

**Production:** a GCE `e2-micro` VM (1 GiB RAM, no swap) behind a Cloudflare tunnel and Cloudflare Access.
The origin has no public port.

**Owner's goal** (from the audit that started the session): not a job dashboard, but a personal
career-automation system that does all of the following with minimal manual intervention:
- discovers postings by itself
- filters to suitable roles
- prepares applications with the CV and the required answers
- submits only where the owner has explicitly authorized it
- tracks outcomes

**Session inputs:**
- **The audit.** A Turkish architecture and product audit dated 2026-10-27. Its key conclusions:
  - Do not apply the proposed ranking patch as is.
  - Restore the old landing page from `3d3ef77` selectively.
  - The real gaps are source coverage, ATS reliability, submission safety, CV and profile evaluation,
    state, observability and operational automation.
  - CI and CD pieces exist but are not chained into CI → release → deploy → health → rollback.
  - Showing the landing page publicly is a separate auth cutover (Cloudflare Access), not a code change.
- **The ranking patch** `opportunity-platform-ranking-20261026.patch`. It was never applied and was used only as a
  reference.
- **The audit archive** `opportunity-platform-audit.tar.gz`. It is byte-identical to the local working
  tree, except that it omits `.env.example` and `.env.prod.example`.
- **An English "master execution prompt"** with roadmap PR-00…PR-10 and the constraints in section 2.

---

## 2. Constraints in force during the session

From the master prompt and the owner's messages (quoted where they were given verbatim):

- No force-push. No rewriting or deleting valuable Git history.
- Never delete production databases or persistent CV files.
- Never publish secrets or personal applicant information.
- No cloud spending without explicit authorization.
- No real job applications without an explicitly configured and verified authorization policy.
  No employer emails.
- No change to live production authentication that risks lockout.
- No deploying unverified changes. No disabling security gates.
- VM access: "Do not modify production yet. Establish and verify read-only connectivity first."
  "Change nothing in production." "Do not ask me to paste SSH private keys, passphrases, or
  long-lived credentials into chat." "Prefer short-lived credentials or identity federation."
- Latest: **"do not commit or push anything, just do the implementation."** Applies to the work
  after `053693d`.
- Repository `CLAUDE.md`:
  - Ask before installing packages or deleting anything.
  - Never stage `.env`, keys or credentials.
  - Keep `.gitignore` current.
  - Keep solutions minimal ("lazy senior dev": no speculative abstraction, but never skip validation,
    data-loss handling or security).
- Local tests never touch the owner's `opportunity-db` (127.0.0.1:5432). They use a disposable
  `postgres:16-alpine` container: 55432 for tests, 55433 for the browser review instance.
- The owner asked to use the "Ponytail" plugin. Its skill could not be loaded in the session, so its
  `SKILL.md` rules were applied manually.

---

## 3. Session timeline

| # | Owner request | What was done | Result |
| --- | --- | --- | --- |
| 1 | Audit and master prompt: implement PR-00…PR-10, keep a tracker in `docs/implementation/` | Baseline reconciliation, then ten local commits: baseline repairs, ranking v2, landing restore, CI acceptance, pause and cap, failure emails, CD workflow, trackers | 282/282 Rust and 13/13 web tests passing locally; nothing pushed |
| 2 | "utilize the Ponytail plugin" | Skill not loadable, so its rules were applied manually | — |
| 3 | Establish read-only GCP VM access via IAP | `gcloud` was missing, so I asked before installing it. No production writes | Blocked until install |
| 4 | "install the gcloud CLI with Homebrew, check enable-oslogin read-only…, then run the read-only IAP ssh test" | `gcloud-cli` 590.0.1 via Homebrew. `instances describe`: OS Login on, no metadata SSH keys, no VM service account, key already in the OS Login profile. IAP reached sshd | SSH refused: passphrase-protected key not in agent |
| 5–7 | Owner ran `ssh-add` (first in the wrong place, then in the tab Claude opened) | Verified with `ssh-add -l`, then ran `hostname && whoami && uptime` over IAP | **Verified read-only access** (commit `053693d`) |
| 8 | "do not commit or push anything, just do the implementation" | Sources (PR-06), source alerts (PR-09b), off-host backups (PR-10), tracker updates, all uncommitted | 283/283 Rust; acceptance passes |
| 9 | Is it deployed? Why the Cloudflare page instead of the meme landing? Backup approach for e2-micro? Discuss before changing code | Read-only investigation of the repo, the public URL and the VM. No code changes | Section 7 and section 8 |
| 10 | "Start the working tree locally in native mode against a throwaway database" | Throwaway DB on 55433, server on 127.0.0.1:8788 (`ACCESS_MODE=native`), test owner account, seed sources sweeping | Landing, login, logout and signed-out workspace verified (section 14) |
| 11 | This report | — | — |

---

## 4. Repository state

### 4.1 Commits on the branch (oldest first)

| Commit | Message | Files | +/- |
| --- | --- | ---: | --- |
| `06fb29a` | snapshot uncommitted single-screen product work (**owner's WIP**, committed unchanged) | 63 | +1743 / −4851 |
| `7f00fc2` | fix stale baseline tests, formatting and pdf extraction in images | 6 | +56 / −22 |
| `10addad` | add implementation tracker and baseline inventory | 6 | +119 |
| `f0744e8` | rank by technical fit, profile and career stage | 7 | +543 / −25 |
| `2588c32` | restore the original landing page with landing, login and workspace routes | 7 | +376 / −11 |
| `ec27d2e` | run the product journey offline in ci | 3 | +80 / −7 |
| `02371d7` | add an owner pause and a global daily cap on automatic submissions | 8 | +152 / −1 |
| `6f87dcb` | email failed, uncertain and manual-completion outcomes | 4 | +67 / −23 |
| `b665d94` | deploy master automatically after ci, refusing schema changes | 2 | +125 |
| `a115ad9` | update implementation tracker, test evidence and handoff | 5 | +65 / −15 |
| `a061fc7` | record vm access check results | 2 | +2 / −3 |
| `053693d` | record verified read-only vm access | 3 | +4 / −4 |

- Session commits `06fb29a..053693d`: 31 files, +1562 / −84.
- Uncommitted: 10 modified files, +220 / −8, plus 2 new files (58 lines).

### 4.2 `git status --short` (working tree)

```
 M config/sources.seed.toml
 M crates/core/src/mail.rs
 M crates/core/src/product.rs
 M crates/core/tests/applications.rs
 M docs/DEPLOYMENT.md
 M docs/implementation/BLOCKERS.md
 M docs/implementation/HANDOFF.md
 M docs/implementation/TASK_STATE.md
 M docs/implementation/TEST_RESULTS.md
 M scripts/deploy-vm.sh
?? .github/workflows/backup.yml
?? migrations/0009_source_alerts.sql
```

Generating this report added one line to `.gitignore` (`/docs/implementation/review/`, in the existing
"Internal reviews and deployment records" section), so `git status` now also shows ` M .gitignore`.
The bundle itself is ignored. The
gitignored local files `.claude/launch.json` (a run configuration) and `.claude/review-login.txt`
(local test login) are not part of the change.

### 4.3 Migrations

`0001`–`0005` are in production. This branch adds four, and **none has been applied to production**:

| Migration | Origin | Effect |
| --- | --- | --- |
| `0006_cv_journey.sql` | owner WIP (`06fb29a`) | `documents.candidate`, `product_settings`, policy `profile_id`, more application statuses, `review_tasks.options`, notification delivery columns, `applications.auto_resume` |
| `0007_email_attempt_window.sql` | owner WIP | `notifications.first_attempt_at` |
| `0008_brakes_and_notifications.sql` | session (`02371d7`, renamed in `6f87dcb`) | `product_settings.submissions_paused`, `max_automatic_per_day` (default 10, 0–100); notification kind `manual_required` |
| `0009_source_alerts.sql` | session, **uncommitted** | notification kind `source_failing` |

`0008` was created as `0008_submission_brakes.sql` and renamed to `0008_brakes_and_notifications.sql`
within the branch. This is safe only because it was never applied anywhere persistent.

---

## 5. Changelog

### PR-00: Baseline reconciliation (`06fb29a`, `7f00fc2`), TESTED

- **Snapshot.** The local tree held large uncommitted owner work:
  - a single-screen product
  - the CV journey
  - five career profiles
  - a mail outbox
  - provider fixtures

  It also deleted the landing, Discover, Matches, Profiles, Sources and Workspace pages. It was
  committed unchanged as `06fb29a` (DECISIONS 1).
- **Baseline failures (3 of 273 tests):**
  - Two tests still expected 4 seeded profiles; the product now seeds 5.
  - A CV-versioning test uploaded a fake PDF that the new text extraction rightly rejects. It now uses
    a synthetic `.txt` CV.
- **`cargo fmt`:** the WIP had not been formatted, so I formatted it.
- **Production bug:** the runtime Docker images had no `pdftotext`, so every PDF CV upload in a container
  was refused. Fix: `poppler-utils` added to the `Dockerfile` runtime stages.
- **Web `node_modules`:** it was broken locally (`.bin` links); fixed with `npm ci`. No dependency changes.

### PR-03: Ranking v2, `career-priority-v2` (`f0744e8`), TESTED

File: `crates/core/src/product.rs` (`RANKING_POLICY` at line 68, `career_stage` at line 174,
`priority_score` at line 247). Policy doc: `docs/RANKING_POLICY.md` (Appendix E).

Order of operations:
1. **Candidate set by SQL prefilter.** Only open opportunities whose `search_vector` matches the
   profile's direction terms are loaded (`websearch_to_tsquery('simple', …)`). This fixes truncation:
   with 2,470 open opportunities locally and a 2,000 `MATCH_SHORTLIST` cap on e2-micro, about 470
   postings were never scored (DECISIONS 4).
2. **Technical fit (0–100).** Built from direction terms in the title (or two in the body), CV skills,
   and CV projects and employment, minus 10 per gap. Postings below 20 are dropped, and no stage bonus
   can rescue them.
3. **Hard exclusions.** The saved profile's job kinds, location preferences and global or per-profile
   blacklist rules apply, via `opportunity_domain::evaluate`. Before this change they were ignored on
   the dashboard.
4. **Review gaps.** Any of the following marks a posting `needs_review`; unknown is never treated as
   eligible:
   - experience years
   - a required PhD
   - work-authorization wording
   - title-only listings
   - PhD posts without a confirmed vacancy
5. **Priority** = `t·technical + p·profile + s·stage`, clamped to 0–100. Weights:
   - engineering profiles: 0.55 / 0.15 / 0.30
   - research profile: 0.45 / 0.20 / 0.35

   The stage table puts thesis, internship and student roles above junior, then mid, then senior and
   staff. The research profile ranks advertised PhDs (95) above internships (85).
6. **Bug fixed.** The research profile's direction terms were stage words, so *any* internship
   (including marketing) got 40 technical points. It now uses the four technical areas' terms
   (DECISIONS 6).
7. **UI.** It shows technical fit, career stage and profile match separately. The dashboard exposes
   `ranking_policy`.
8. **Deliberately not done:** experience years from employment date ranges (DECISIONS 5, PR-05b).

**Tests:**
- 8 unit tests
- an acceptance stage-order check on a synthetic corpus
- a check that an unrelated internship is excluded

### PR-04: Landing page restore (`2588c32`), TESTED

- **Restored from `3d3ef77`:**
  - `LandingPage.tsx`
  - `linkedin-vibe.webp` (the "meme")
  - its styles
  - `Landing.test.tsx` (expanded to 8 cases)

  The copy is the original: "Alp's Opportunity Platform", "Log in as Alp (alpblkba)", "Log in as another
  user" (a Rick Roll link), "back to home", "what is this?" and "bro casually replacing Linkedin".
- **Routes:**
  - `/` is public and asks only `/api/auth/session` whether a session exists.
  - `/login` is the form.
  - `/workspace` and every other path is the app, which renders the login form when there is no session.
- **Identity handling:**
  - Cloudflare identities go straight to the workspace.
  - A missing identity layer offers a reload instead of a dead-end password form.
- **Fixes during the work:**
  - Logout used to set an "expired" state, because `act()` reloaded `/home`; it now has its own handler.
  - Home is no longer fetched while on the landing page.
  - A light-palette media-query wrapper had been dropped by a filter and was restored.
- **Checked** in a browser at desktop and 375 px, in light and dark, with no horizontal overflow.

### PR-01: Deterministic acceptance journey in CI (`ec27d2e`), TESTED locally

- **New job** `acceptance` in `.github/workflows/hygiene.yml`. It uses a PostgreSQL service, builds the
  server, the web app and the worker, installs Playwright Chromium, and runs
  `TEST_OFFLINE=1 node scripts/test-product.mjs`.
- **What the journey covers:**
  - landing
  - login
  - CV upload
  - ranking order
  - restart
  - employer questions
  - mock ATS submission
  - duplicate prevention
  - email via a loopback mail fixture
- **`TEST_OFFLINE=1`** disables every seed source, asserting 0 `source_runs`, so there is no network access.
- **CV version check:** the submission's `cv_document_id` must be the CV version that was current at
  submission (PR-05).
- **Not yet run in GitHub Actions** (branch not pushed).

### PR-07/08: Owner pause and global daily cap (`02371d7`), TESTED

- **Settings:** `product_settings.submissions_paused` and `max_automatic_per_day` (default 10).
- **Enforcement point:** both are checked in `workflow::submit` (`crates/core/src/workflow.rs:239`),
  the single path every real submission takes (DECISIONS 8). The cap check sits *inside* the browser
  slot, which already serializes submissions, so two concurrent submissions cannot both pass it.
- **While paused,** the queue prepares nothing (`product.rs` tick).
- **API, UI and tests:**
  - `pause_submissions` (`product.rs:586`)
  - dashboard fields `submissions_paused` and `max_automatic_per_day`
  - a UI test
  - integration test `the_owner_pause_and_daily_cap_refuse_submissions_above_every_policy`

### PR-09: Failure, uncertain and manual-completion emails (`6f87dcb`), TESTED

- **Bug:** `failed` and `submission_uncertain` notifications were stored but never emailed (dispatch
  only sent `submitted` and `action_required`).
- **Fix:**
  - All kinds are now dispatched.
  - `manual_required` is notified once per application.
  - Each email says what to do next (`mail::wording`).
  - Migration `0008` extends the kind check.

### PR-02: Automated CD (`b665d94`), IMPLEMENTED (never run)

`.github/workflows/deploy.yml`:
- **Trigger:** `workflow_run` after `continuous integration` succeeds on a **push to `master`**, or a
  manual dispatch on master.
- **Gates:**
  - It is inert until `vars.DEPLOY_ENABLED == 'true'`.
  - It uses `environment: production`.
  - Concurrency group `production`, never cancelling in progress.
- **Steps:**
  - checkout of the exact SHA
  - **schema gate:** an automatic run fails if `migrations/` changed since the last successful deploy run
  - GCP auth via Workload Identity Federation (no keys)
  - `release-image.sh`
  - `deploy-vm.sh --preflight`, then `deploy-vm.sh <archive>`, then `--status`
- **No registry:** it reuses the archive-over-IAP release path the scripts already verify
  (DECISIONS 9).
- **Bootstrap:** the one-time owner setup (WIF pool and provider, `opportunity-deploy` SA with
  `compute.viewer`, `iap.tunnelResourceAccessor` and `compute.osAdminLogin`, the GitHub `production`
  environment, and repository variables) is documented in `docs/DEPLOYMENT.md` "Automated deployment"
  (Appendix D).

### Tracker docs (`10addad`, `a115ad9`, `a061fc7`, `053693d`)

`docs/implementation/`: `MASTER_PLAN`, `TASK_STATE`, `DECISIONS`, `TEST_RESULTS`, `BLOCKERS`,
`DEPLOYMENT_READINESS` and `HANDOFF`. Their current text is in Appendix E.

### OPS: Read-only VM access, VERIFIED_COMPLETE

- **Setup:**
  - Installed `gcloud-cli` 590.0.1 with Homebrew, after the owner approved it.
  - Checked read-only first: OS Login is enabled on the instance, there are no SSH keys in instance or
    project metadata, and no service account is attached.
- **Effect:** `gcloud compute ssh` therefore writes nothing to the VM or the project. The owner's
  public key was already in their OS Login profile.
- **The owner loaded the passphrase-protected `~/.ssh/google_compute_engine` key into ssh-agent**
  in their own terminal. The passphrase never went through chat, and it is now in the macOS keychain.
- **Result** of `hostname && whoami && uptime` over IAP: the instance name, the OS Login user, and
  "up 4 days, 2 users, load 0.00". The "2 users" was not investigated.
- **Local-only writes:** the ssh-agent, the keychain and gcloud's known-hosts file.

### Uncommitted work (after "do not commit or push anything")

#### PR-06: Source coverage, TESTED

`config/sources.seed.toml` gains five sources. Each was added only after a real sweep through the app's
own adapters into a scratch DB:

| Source | Adapter | Sweep result |
| --- | --- | --- |
| `extropic-ashby` | Ashby | `ok_complete`, 10 postings |
| `intel-workday` | Workday (`intel`, `wd1`, `External`) | `partial`, hit the 500-posting ceiling (`MAX_PAGES` 25 × `PAGE_SIZE` 20) |
| `micron-workday` | Workday (`micron`, `wd1`, `External`) | `partial` at 500 |
| `marvell-workday` | Workday (`marvell`, `wd1`, `MarvellCareers`) | 340 stored, but reported `partial` (existing Workday paging quirk, not fixed) |
| `cadence-workday` | Workday (`cadence`, `wd1`, `External_Careers`) | `partial` at 500 |

- **Not added:**
  - **Anthropic and OpenAI:** a single response exceeds the HTTP client's 8 MiB body cap
    (`crates/adapters/src/client.rs:65`), so every sweep would be `http_error`.
  - **No answer at the tried endpoints:** AMD, Qualcomm, Arm, Ampere, Synopsys, Groq, SiFive and others.
- **Production effect:** the new rows reach production on the next deploy. `serve` calls
  `product::initialize` (`crates/cli/src/main.rs:269`), which imports the seed compiled into the
  binary (`include_str!`). It inserts new keys and skips existing ones, so owner edits are kept.

#### PR-09b: Alert on sources that keep failing, TESTED

- **Migration:** `migrations/0009_source_alerts.sql` adds `source_failing` to the notification kind check.
- **Detection:** `product::alert_failing_sources` (`crates/core/src/product.rs:776`) runs on every
  scheduler tick, before `mail::dispatch`. Any enabled source with `consecutive_failures >= 3` (after the
  scheduler's own retries) creates a notification.
- **Dedupe:** the dedupe key is `source-failing:{source_id}:{last_success_at epoch or 0}`, which gives
  one alert per outage and a new one after a recovery followed by another failure.
- **Mail:** dispatch now `left join`s applications, opportunities and organizations, so a notification
  with no application still renders. The employer falls back to `payload.source`, the position to
  "career site", and the URL to `''`. New wording: "Source not refreshing / Its listings are kept, not
  closed, while it fails…".
- **Test:** `a_source_that_keeps_failing_alerts_the_owner_once_per_outage`:
  - 2 failures → 0 alerts
  - 3 failures, checked twice → 1 alert
  - recovery, then 4 failures → 2 alerts

  The mail unit test now expects 6 distinct subjects.

#### PR-10: Off-host encrypted backups, IMPLEMENTED (not run against the VM)

- **`scripts/deploy-vm.sh --fetch-backup`** (function `fetch_backup`, line 280). Over the same IAP
  transport as deploys, it streams two things straight into `gpg --encrypt --recipient
  $BACKUP_ENCRYPT_TO` on the machine that runs it:
  - `pg_dump --format=custom --no-owner` from the `db` container
  - `tar -czf - -C /app private` (CVs and browser evidence) from the `app` container

  **Nothing is written on the VM.** Plaintext exists only in the pipe. After writing, it checks:
  - that the recipient key exists
  - that each file is larger than 1024 bytes (an encrypted empty stream is a few hundred bytes)
  - and it writes a `sha256` file per output

  `--dry-run` prints the remote commands.
- **`.github/workflows/backup.yml`** (new): daily at 03:17 UTC plus manual dispatch. It is inert until
  `BACKUP_ENABLED`. It uses the deploy WIF identity, imports the owner's *public* key from a variable,
  runs `--fetch-backup`, then `gcloud storage cp --no-clobber` to a private bucket. The bucket binding is
  `objectCreator` only, so the workflow cannot read or delete objects.
- **`docs/DEPLOYMENT.md`:** new "Automated off-host backups" section covering bucket creation with
  public-access prevention, a 90-day lifecycle, the IAM binding, the `gh variable` commands and a
  restore rehearsal.
- **`scripts/restore.sh`:** already accepts `.dump.gpg` with its ciphertext `.sha256`.
- **Verified locally:**
  - The same `pg_dump` piped into `gpg` against a migrated scratch DB, encrypted to a throwaway key,
    decrypted to an archive with 54 restorable tables.
  - A failing dump exits non-zero.
  - `--dry-run` prints the expected commands.
- **Owner's latest position** (turn 9) changes this. See section 8.5: manual pull to the Mac first, no
  automated backups yet. `backup.yml` may be dropped.

---

## 6. Test evidence (all local; nothing in GitHub Actions)

Environment: macOS, Rust 1.92, Node 26, disposable PostgreSQL 16 container.

| When | Check | Result |
| --- | --- | --- |
| Baseline `06fb29a` | `cargo test --workspace` | 270 passed, **3 failed** (stale tests, fake PDF fixture) |
| Baseline | `cargo fmt --check` | failed (WIP formatting); clippy OK |
| Baseline | web typecheck, Vitest | OK, 3/3 (after `npm ci`) |
| Baseline | `scripts/verify-browser-worker.sh` | 19/19 |
| `7f00fc2` | fmt, clippy `-D warnings` | OK; `native_auth` 13/13, `applications` 23/23, `discovery` 43/43 |
| `6f87dcb` / `b665d94` | `cargo test --locked --workspace --no-fail-fast` | **282/282** |
| same | web typecheck, Vitest | OK, **13/13** (Home 5, Landing 8) |
| same | `TEST_OFFLINE=1 node scripts/test-product.mjs` | three PASS lines, `browserErrors: []`, 0 `source_runs` |
| same | `scripts/test_release.py` / `test_ci.py` / `test_privacy.py` / `check_privacy.py` | 44 OK / 14 OK / OK / PASS |
| same | landing in browser, desktop and 375 px, light and dark | no overflow |
| Uncommitted tree | Rust | **283/283**, fmt and clippy clean |
| Uncommitted tree | web / worker / offline acceptance | 13/13, 19/19, three PASS lines, `browserErrors: []` |
| Uncommitted tree | real sweeps into a scratch DB | as in the PR-06 table |
| Uncommitted tree | backup pipe round-trip | 54 restorable tables; failing dump exits non-zero |
| Review instance (turn 10) | API auth checks, see section 14 | all as expected |

**Not run anywhere:**
- GitHub Actions: `hygiene`, `deploy` or `backup`
- `actionlint` (CI runs it)
- `release-image.sh`, which needs a clean checkout of a reviewed SHA and a full linux/amd64 Docker build
- any deploy, backup or command against the VM beyond the read-only checks in section 7
- native amd64 on 1 GiB

---

## 7. Production state (read-only checks, 2026-10-27)

Public URL:

```
$ curl -s -o /dev/null -w '%{http_code} %{redirect_url}' https://opportunities.alpblkba.dev/
302 https://damp-math-e853.cloudflareaccess.com/cdn-cgi/access/login/opportunities.alpblkba.dev?kid=…&redirect_url=%2F
```

VM, over IAP (`docker ps`, `/health`, `printenv ACCESS_MODE`, `_sqlx_migrations`, `df`, `free`,
`docker system df`, database size). Nothing was changed:

```
== containers
opportunity-app-1     opportunity-app:1043ab9491d4b8815835dae27e83fdea73533b0b-amd64  Up 2 days (healthy)
opportunity-tunnel-1  cloudflare/cloudflared:latest  Up 4 days
opportunity-db-1      postgres:16-alpine  Up 4 days (healthy)
== version        {"status":"ok","version":"0.1.0"}
== access mode    cloudflare
== migrations     5
== disk           /dev/sda1  25G  3.4G  20G  15% /
== memory         total 964 MB, used 594, free 145, buff/cache 375, available 370; swap 0
== backups dir    none under /opt/opportunity/backups; .release-history 39M
== docker disk    images 716.9MB (42MB reclaimable), volumes 49.8MB
== db size        9095 kB
```

**What this means:**
- **Nothing from this branch is deployed.** Production runs `master` (`1043ab9`), schema 5, and
  `ACCESS_MODE=cloudflare`.
- **The landing page is not new code.** The meme landing page is already in `1043ab9`, and so is native
  auth (migration `0005`). Every visitor still sees Cloudflare's Access login (team `damp-math-e853`),
  because one Access application covers the whole hostname.
- **`vm-release.sh` takes no database backup itself.** The "recovery point" in a deploy is the retained
  previous image and configuration, not a database dump.
- **The tunnel image is pinned to `cloudflare/cloudflared:latest`.** Noted, not changed.

---

## 8. Deployment strategy

### 8.1 Release path (existing scripts, unchanged in this session)

1. `REVIEWED_SHA=<full sha> ./scripts/release-image.sh` builds a linux/amd64 runtime image archive. It
   refuses a dirty tree, refuses `REVIEWED_SHA != HEAD`, and refuses if the checkout changes during the
   build. **A deploy therefore always needs a commit; it does not need a push.**
2. `./scripts/deploy-vm.sh --preflight <archive>` checks everything and changes nothing.
3. `./scripts/deploy-vm.sh <archive>` does the following, then `--status` reports the running release.
   Exit 1 means recovery passed; exit 2 means unresolved state.
   - copies the archive over IAP
   - runs `vm-release.sh`: durable journal lock, retain the previous immutable image, load, start
   - checks health, readiness, identity and mode via `vm-check.sh`
   - rolls back the image and configuration on failure
4. Secrets live only in `/opt/opportunity/.env.prod` on the VM.

### 8.2 Recommended first deploy of this branch (manual; schema changes block the automatic path)

**Why manual:** migrations `0006`–`0009` run on start. An older image refuses a newer migration history
("migration N was previously applied but is missing"), so after this deploy, **image rollback alone
cannot return to `1043ab9`**. Going back would need a restore of the pre-deploy backup into a new
database.

1. The owner reviews the code (this report) and tests locally in the browser (section 14).
2. Settle the open backup decisions (section 8.5), then commit the uncommitted work.
3. Recommended: push the branch and open a PR, so `hygiene`, including the new `acceptance` job, runs
   in Actions for the first time. Then merge to `master`, so production runs a commit on `master`.
4. Take a backup to the owner's Mac:
   ```bash
   BACKUP_OUT_DIR=~/Backups/opportunity BACKUP_ENCRYPT_TO=<key fingerprint> ./scripts/deploy-vm.sh --fetch-backup
   ```
   Then rehearse a restore into a throwaway database:
   ```bash
   ./scripts/restore.sh <file>.dump.gpg rehearsal_db
   ```
5. On a clean checkout of the reviewed SHA, run `release-image.sh`, then `deploy-vm.sh --preflight`,
   then `deploy-vm.sh <archive>`, then `deploy-vm.sh --status`.
6. Verify in the browser behind Cloudflare Access. `ACCESS_MODE` stays `cloudflare` during this release.
7. Only afterwards, and as a separate owner-authorized change, do the auth cutover (section 8.4).
8. The five new sources are added automatically when the new binary starts (section 5, PR-06).

### 8.3 Automated CD (after bootstrap)

Push to `master` → `continuous integration` green → `deploy.yml` (only if `DEPLOY_ENABLED`):
- Releases that touch `migrations/` since the last successful deploy run are refused and must be run with
  **Actions → deploy → Run workflow** after a backup.
- The first workflow deploy must be a manual run, to set the baseline revision.

Owner bootstrap (commands in Appendix D):
- the WIF pool and provider, restricted to repository `alpblkba/opportunity-platform` and environment
  `production`
- the `opportunity-deploy` SA and its three roles
- the GitHub `production` environment limited to `master`
- the `GCP_*` and `DEPLOY_ENABLED` variables

### 8.4 Showing the custom landing page instead of the Cloudflare page (auth cutover)

**Root cause:** the Cloudflare Access application protects all of `opportunities.alpblkba.dev`.
The app's own landing and login are unreachable until Access stops gating the hostname. This needs no
code change; it is the ladder in `docs/AUTHENTICATION.md` "The Cloudflare cutover":

1. Create the owner account on the VM:
   ```bash
   opportunity user create --email … --name …
   ```
   The password is entered at a prompt, never in argv or chat.
2. `ACCESS_MODE=both`, then restart: Cloudflare *and* a native session are required. This proves native
   login in production with nothing opened.
3. `ACCESS_MODE=native`, then restart, then `vm-check.sh`. That must report the container really runs
   `native`.
4. The owner, in Zero Trust:
   - Replace the owner-only **Allow** policy with **Bypass / Everyone**. Keep the application object, so
     the audience tag is kept and rollback is a single policy swap.
   - Add a Cloudflare rate-limit rule on `/api/auth/login` (about 10 requests per minute per IP, with a
     managed challenge).
5. **Rollback:** put the Allow policy back first, then set `ACCESS_MODE=cloudflare`. Never the reverse
   order.

**Why not path-scoped Bypass** (open only `/`): Access would intercept the app's `/api` XHRs after a
native login. Bypass is evaluated before Allow, and is scoped by application destination.

**Trade-off (open owner decision):** Cloudflare's emailed one-time code is currently an effective second
factor. After the cutover, the password (Argon2id, rate-limited, Origin-checked, 12-hour sessions) is the
only factor. The options are to accept that, or to add TOTP to native login before the cutover.

### 8.5 Backups: implemented vs. the owner's preferred approach (open decision)

**Owner's proposal (turn 9):** on a small VM, run the backup on the VM, scp it to the owner's Mac when the
Mac is reachable (abort otherwise), then delete it from the VM. No automated backups at first.

**Analysis:**
- `--fetch-backup` already satisfies the goal, with fewer steps. It is *pulled* by the Mac over IAP and
  streams straight to a file on the Mac. Nothing is written to the VM's disk, so nothing needs copying
  or deleting.
- If the Mac is off, it simply doesn't run. If the connection drops, the run fails.
- A VM-to-Mac push would require the Mac to be reachable from the internet and a Mac credential stored
  on the VM.
- **Scale:** the database is 9 MB, the VM has 20 GB of free disk, and there are no backup files on the VM.

**Proposed follow-ups** (not done; awaiting the owner):
- Drop `.github/workflows/backup.yml` and the GCS section of `DEPLOYMENT.md` from this change.
- Delete partial output files when a stream fails mid-way (section 12, item 2).
- Decide on encryption: keep GPG (recommended, since the dump contains the CV and application history),
  or allow plaintext on a FileVault disk that is not cloud-synced.
- Later, optionally, a macOS `launchd` job that runs `--fetch-backup` daily while the Mac is awake.

### 8.6 Rollback matrix

| Failure | Recovery |
| --- | --- |
| Deploy health or readiness fails | `deploy-vm.sh` restores the previous image and configuration automatically (exit 1 = proven) |
| Bad release after migrations `0006`–`0009` applied | image rollback is **not** enough; restore the pre-deploy backup into a new database and point the old image at it, keeping the migrated database for investigation |
| Cutover step 4 opened too much | swap Bypass back to the owner-only Allow; immediate, no deploy |
| Native login broken after step 4 | Allow policy back first, then `ACCESS_MODE=cloudflare` and restart |
| Steps 2–3 misbehave | `ACCESS_MODE=cloudflare` and restart; Cloudflare unchanged |

---

## 9. Task state (consolidated, current)

| ID | Task | Status | Evidence / blocker |
| --- | --- | --- | --- |
| PR-00 | Baseline snapshot, stale tests, fmt, pdftotext in images | TESTED | `06fb29a`, `7f00fc2` |
| PR-01 | Deterministic acceptance journey in CI | TESTED | `ec27d2e`; offline journey passes locally, 0 source runs; never run in Actions |
| PR-03 | Ranking v2 | TESTED | `f0744e8`; 8 unit tests + acceptance stage order; `docs/RANKING_POLICY.md` |
| PR-04 | Landing page restore | TESTED | `2588c32`; 8 Vitest cases; visual check; re-verified on the local review instance |
| PR-05 | CV version per application | TESTED | acceptance asserts the submission's `cv_document_id` |
| PR-05b | Experience years from date ranges | NOT_STARTED | deliberately deferred (DECISIONS 5) |
| PR-07/08 | Owner pause and global daily cap | TESTED | `02371d7`; integration + UI tests |
| PR-09 | Failure / uncertain / manual-completion emails | TESTED | `6f87dcb` |
| PR-02 | CD workflow | IMPLEMENTED | `b665d94`; needs bootstrap; never run |
| PR-06 | Source coverage expansion | TESTED (uncommitted) | 5 sources, each with a real sweep |
| PR-09b | Alert on sources that keep failing | TESTED (uncommitted) | migration `0009`, integration test |
| PR-10 | Off-host encrypted backups | IMPLEMENTED (uncommitted) | pipe round-tripped locally; never run against the VM; scope under owner review (section 8.5) |
| PR-08b | Unattended worker on a larger VM | BLOCKED | VM size/cost approval; owner decision on `BROWSER_WORKER_ALLOW_EXTERNAL` |
| OPS | Read-only VM access | VERIFIED_COMPLETE | IAP + OS Login, no production writes |
| AUTH | Public landing page (Cloudflare cutover) | NOT_STARTED | owner decision (TOTP or password-only), then section 8.4 |

---

## 10. Decisions (from `DECISIONS.md`)

1. Snapshot the owner's uncommitted tree as its own commit on a feature branch.
2. Keep the stack: Rust/Axum, React, PostgreSQL, Playwright, Compose, Actions, GCE and Cloudflare.
3. Keep the browser worker's external-navigation guard. Unattended real submission stays an explicit
   owner switch.
4. Fix shortlist truncation with a SQL prefilter on direction terms, rather than raising the cap or
   caching.
5. Experience years stay explicit-claim only. Summing date ranges would let telecom years satisfy an RTL
   "3+ years" requirement.
6. The research profile uses the four technical areas as its direction terms.
7. Ranking weights live in code, versioned by `RANKING_POLICY`. No runtime config yet.
8. Pause and cap live in `workflow::submit`, with the cap checked inside the browser slot.
9. No container registry: CD reuses archive-over-IAP. Schema-changing releases are manual.

Added in this session's discussion (not yet in `DECISIONS.md`):
- No staging copy on the e2-micro (370 MB available, no swap); browser review happens locally.
- Backups are pulled by the owner's Mac, not pushed from the VM.

---

## 11. Blockers and open owner decisions

| Item | Needed for | Owner action |
| --- | --- | --- |
| Review, then permission to commit the uncommitted work | everything below | review; say "commit" |
| Push / PR | first real CI run | explicit go-ahead (currently "do not commit or push") |
| TOTP vs. password-only | auth cutover | decision |
| Backup encryption (GPG vs. plaintext); drop `backup.yml`? | PR-10 scope | decision; a GPG key on the Mac if encrypted |
| WIF pool/provider, deploy SA, `production` environment, `DEPLOY_ENABLED` | automated CD | GCP and GitHub bootstrap (Appendix D) |
| Cloudflare policy swap and rate-limit rule | public landing page | Zero Trust dashboard |
| Larger VM (2–4 GiB) | unattended applications | cost approval |
| Real submission policy | live applications | enable an employer authorization in the app |

---

## 12. Known issues, risks and review focus

1. **Deploy schema gate assumption** (`.github/workflows/deploy.yml:57`, marked `shortcut:`). It
   takes the last run with `--status success` as "last deployed revision". If GitHub reports a run whose
   job was skipped (for example `DEPLOY_ENABLED` unset) as `success`, the gate compares against a
   revision that was never deployed. Confirm on the first real runs. A more robust alternative is to
   read the deployed SHA from the VM (`deploy-vm.sh --status`).
2. **Partial backup files** (`scripts/deploy-vm.sh` `fetch_backup`). Under `set -euo pipefail`, a
   stream that fails mid-way makes the function fail, but the half-written `.gpg` file stays in
   `BACKUP_OUT_DIR` without a `.sha256`. `restore.sh` would refuse it, because it has no checksum, but it
   should still be deleted.
3. **`gpg --trust-model always`** in `fetch_backup` and `backup.yml`. The recipient is whatever key
   matches `BACKUP_ENCRYPT_TO` in the local or runner keyring; a fingerprint is expected, not a name or
   email. Worth validating that the value is a 40-hex fingerprint.
4. **Submission cap concurrency** (`crates/core/src/workflow.rs:239`). The correctness argument is that
   the count runs inside the browser slot (an advisory lock) that serializes submissions. Reviewers
   should confirm no submission path bypasses the slot. `min(max_automatic_per_day)` across
   `product_settings` rows is global for the single owner.
5. **Source-alert dedupe key** (`product.rs:776`). It uses `last_success_at` (or 0). A source that
   has *never* succeeded alerts once, ever, until its first success. The check runs every scheduler tick;
   the query is small (enabled sources only).
6. **Mail `left join`.** Notifications without an application now render with fallbacks. Check that
   no existing kind relied on the inner join to filter rows out.
7. **Ranking weights and the stage table are uncalibrated** (an initial policy). The title and kind
   heuristics in `career_stage` decide internship, junior and senior; misclassification shifts priority
   by up to about 30 points.
8. **Workday:**
   - The 500-posting ceiling makes large tenants permanently `partial`. That is by design: `partial`
     never closes postings, but also never reports complete coverage.
   - Marvell is reported `partial` despite 340 of 340 being stored, an existing paging-heuristic quirk
     (`crates/adapters/src/workday.rs:100`).
9. **The seed is compiled into the binary.** `product::initialize` imports the `include_str!` copy at
   every `serve`, adding new keys and skipping existing ones. A source the owner deleted on purpose is
   therefore re-created at the next start, unless it was disabled rather than deleted. `run.sh` has its
   own "only if no sources" import, which is redundant with this.
10. **8 MiB body cap.** It excludes Anthropic and OpenAI boards. The fix is a paged or title-only fetch,
    not raising the cap on a 1 GiB host.
11. **The migration rename within the branch** (`0008_submission_brakes` → `0008_brakes_and_notifications`).
    It is harmless only because it was never applied to a persistent DB. Anyone who ran the branch at
    `02371d7` against a kept database would need to reset it.
12. **Auth cutover risk.** After Bypass, the password is the only factor. Per-IP in-memory rate limiting
    (512 addresses, a 15-minute ceiling) is the origin's own defence; the Cloudflare rate-limit rule is
    recommended before step 4.
13. **Stale doc.** `docs/implementation/DEPLOYMENT_READINESS.md` (Appendix E) still says there is no VM
    access, that backups are manual, and lists schema changes as `0006`–`0008`. Section 7 and section 8 of
    this report are current. `DEPLOYMENT.md` "Known limitations" says "no rate limiting at the origin",
    which predates native auth's login limiter.
14. **The `cloudflared:latest` tunnel image** in production is unpinned (pre-existing).
15. **"2 users" logged in** on the VM at the first access check: not investigated.

---

## 13. Things explicitly *not* done

- No push, no PR, no GitHub Actions runs, no deploy, no production change of any kind.
- No commit of the work after `053693d` (owner instruction).
- No production backup fetched: it would move personal data and needs the owner's key.
- No Cloudflare or GCP IAM change; no `.env.prod` read.
- No real application submitted; no employer emailed.
- The ranking patch was not applied; it was used as a reference.
- PR-05b (experience years from date ranges) was deferred deliberately.
- PR-08b (unattended worker) is blocked on the owner.

---

## 14. Local review instance (running at the time of writing)

**Started on the owner's request (turn 10)** so the working tree can be tested in a browser before
committing:

| | |
| --- | --- |
| URL | `http://localhost:8788` (127.0.0.1 only) |
| Mode | `ACCESS_MODE=native`, `APPLICATION_ENV=local`, no mail key (no email sent) |
| Database | throwaway container `opportunity-review-db`, `127.0.0.1:55433`, started with `--rm` |
| Binary | `target/debug/opportunity` built from the working tree; `apps/web/dist` rebuilt |
| Data | migrations `0001`–`0009`; 19 seed sources imported and sweeping public boards (411 opportunities after about a minute) |
| Account | test owner `owner@example.test`; the generated password is only in the git-ignored, mode-600 `.claude/review-login.txt` (not reproduced here) |
| Untouched | the owner's own local server on 8787 and their `opportunity-db` on 5432 |

API checks against the review instance:

```
session before: 401
wrong password: 401
login: 200
session after: {"email":"owner@example.test","display_name":"Review Owner","role":"owner","via":"password"}
api without cookie: 401
home with cookie: 200
after logout: 401
```

Browser checks:
- `/` renders the restored landing page with the meme.
- `/workspace` signed out shows "Sign in to your job search." with email and password fields.
- No server errors were logged. The console's three 401s are the expected signed-out session checks.

**Teardown:**
- Stop the `review-native` preview server.
- Run `docker stop opportunity-review-db`; the container removes itself.
- Delete `.claude/review-login.txt`.

---

## Appendices

- **A.** Raw `git` state
- **B.** Full diff of the session commits `06fb29a..053693d` (binary `linkedin-vibe.webp` shown as binary)
- **C.** Full diff of the uncommitted working tree, plus the two new files
- **D.** `docs/DEPLOYMENT.md` (current working-tree text)
- **E.** `docs/implementation/*` and `docs/RANKING_POLICY.md` (current working-tree text)
- **F.** Owner WIP snapshot `06fb29a`: file stat (full patch in `wip-snapshot.patch`)


## Appendix A: raw git state

~~~~~~text
$ git log --format="%h %ad %s" --date=iso master..HEAD
053693d 2026-10-27 04:56:11 +0100 record verified read-only vm access
a061fc7 2026-10-27 04:41:36 +0100 record vm access check results
a115ad9 2026-10-27 04:36:45 +0100 update implementation tracker, test evidence and handoff
b665d94 2026-10-27 04:36:00 +0100 deploy master automatically after ci, refusing schema changes
6f87dcb 2026-10-27 04:34:26 +0100 email failed, uncertain and manual-completion outcomes
02371d7 2026-10-27 04:30:45 +0100 add an owner pause and a global daily cap on automatic submissions
ec27d2e 2026-10-27 04:27:55 +0100 run the product journey offline in ci
2588c32 2026-10-27 04:24:02 +0100 restore the original landing page with landing, login and workspace routes
f0744e8 2026-10-27 04:20:56 +0100 rank by technical fit, profile and career stage
10addad 2026-10-27 04:14:58 +0100 add implementation tracker and baseline inventory
7f00fc2 2026-10-27 04:12:50 +0100 fix stale baseline tests, formatting and pdf extraction in images
06fb29a 2026-10-27 04:09:59 +0100 snapshot uncommitted single-screen product work

$ git status --short
 M .gitignore
 M config/sources.seed.toml
 M crates/core/src/mail.rs
 M crates/core/src/product.rs
 M crates/core/tests/applications.rs
 M docs/DEPLOYMENT.md
 M docs/implementation/BLOCKERS.md
 M docs/implementation/HANDOFF.md
 M docs/implementation/TASK_STATE.md
 M docs/implementation/TEST_RESULTS.md
 M scripts/deploy-vm.sh
?? .github/workflows/backup.yml
?? migrations/0009_source_alerts.sql

$ git diff --stat 06fb29a HEAD   # session commits
 .github/workflows/deploy.yml                 |  78 ++++
 .github/workflows/hygiene.yml                |  51 +++
 Dockerfile                                   |   6 +-
 README.md                                    |   2 +-
 apps/web/public/linkedin-vibe.webp           | Bin 0 -> 22878 bytes
 apps/web/src/App.tsx                         |  42 +-
 apps/web/src/__tests__/Home.test.tsx         |  17 +-
 apps/web/src/__tests__/Landing.test.tsx      | 122 ++++++
 apps/web/src/api.ts                          |   7 +-
 apps/web/src/components/LandingPage.tsx      |  67 ++++
 apps/web/src/styles.css                      | 154 ++++++++
 crates/api/src/product.rs                    |  12 +
 crates/api/tests/native_auth.rs              |   7 +-
 crates/core/src/mail.rs                      |  35 +-
 crates/core/src/matching.rs                  |   2 +-
 crates/core/src/product.rs                   | 567 +++++++++++++++++++++++++--
 crates/core/src/profiles.rs                  |   2 +-
 crates/core/src/workflow.rs                  |  50 +++
 crates/core/tests/applications.rs            | 111 +++++-
 crates/core/tests/discovery.rs               |   8 +-
 docs/DEPLOYMENT.md                           |  47 +++
 docs/RANKING_POLICY.md                       |  49 +++
 docs/implementation/BLOCKERS.md              |   9 +
 docs/implementation/DECISIONS.md             |  21 +
 docs/implementation/DEPLOYMENT_READINESS.md  |  12 +
 docs/implementation/HANDOFF.md               |  34 ++
 docs/implementation/MASTER_PLAN.md           |  43 ++
 docs/implementation/TASK_STATE.md            |  21 +
 docs/implementation/TEST_RESULTS.md          |  29 ++
 migrations/0008_brakes_and_notifications.sql |   7 +
 scripts/test-product.mjs                     |  34 +-
 31 files changed, 1562 insertions(+), 84 deletions(-)

$ git diff --stat HEAD   # uncommitted (tracked files)
 .gitignore                          |  1 +
 config/sources.seed.toml            | 61 +++++++++++++++++++++++++++++++++++++
 crates/core/src/mail.rs             |  6 ++--
 crates/core/src/product.rs          | 27 ++++++++++++++++
 crates/core/tests/applications.rs   | 51 +++++++++++++++++++++++++++++++
 docs/DEPLOYMENT.md                  | 30 +++++++++++++++++-
 docs/implementation/BLOCKERS.md     |  2 +-
 docs/implementation/HANDOFF.md      |  6 ++--
 docs/implementation/TASK_STATE.md   |  5 +--
 docs/implementation/TEST_RESULTS.md | 10 ++++++
 scripts/deploy-vm.sh                | 30 ++++++++++++++++++
 11 files changed, 221 insertions(+), 8 deletions(-)

$ git ls-files --others --exclude-standard   # untracked
.github/workflows/backup.yml
migrations/0009_source_alerts.sql
~~~~~~

## Appendix B: full diff of the session commits (`git diff 06fb29a 053693d`)

~~~~~~diff
diff --git a/.github/workflows/deploy.yml b/.github/workflows/deploy.yml
new file mode 100644
index 0000000..d594dc8
--- /dev/null
+++ b/.github/workflows/deploy.yml
@@ -0,0 +1,78 @@
+name: deploy
+
+# Ships a master commit only after "continuous integration" passed on that exact commit. The
+# existing scripts do the work: release-image.sh builds the reviewed archive, deploy-vm.sh copies it
+# over IAP and runs vm-release.sh (journal lock, backup, health checks, rollback). Secrets stay in
+# .env.prod on the VM. Inert until the owner completes docs/DEPLOYMENT.md "Automated deployment"
+# and sets the DEPLOY_ENABLED repository variable.
+on:
+  workflow_run:
+    workflows: [continuous integration]
+    types: [completed]
+    branches: [master]
+  workflow_dispatch:
+
+permissions:
+  contents: read
+  actions: read
+  id-token: write
+
+# One release at a time; a newer pending run replaces an older pending one.
+concurrency:
+  group: production
+  cancel-in-progress: false
+
+defaults:
+  run:
+    shell: bash
+
+jobs:
+  deploy:
+    name: Build, deploy and verify
+    if: >-
+      vars.DEPLOY_ENABLED == 'true' && (
+        (github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/master') ||
+        (github.event.workflow_run.conclusion == 'success' && github.event.workflow_run.event == 'push')
+      )
+    runs-on: ubuntu-24.04
+    timeout-minutes: 45
+    environment: production
+    env:
+      REVIEWED_SHA: ${{ github.event.workflow_run.head_sha || github.sha }}
+      GCP_PROJECT: ${{ vars.GCP_PROJECT }}
+      GCP_INSTANCE: ${{ vars.GCP_INSTANCE }}
+      GCP_ZONE: ${{ vars.GCP_ZONE }}
+    steps:
+      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
+        with:
+          ref: ${{ env.REVIEWED_SHA }}
+          fetch-depth: 0
+          persist-credentials: false
+      # An older image refuses a newer migration history, so image rollback cannot undo a schema
+      # change. Those releases are deployed deliberately with "Run workflow", never automatically.
+      - name: Stop automatic deploys that change the schema
+        if: github.event_name == 'workflow_run'
+        env:
+          GH_TOKEN: ${{ github.token }}
+        # shortcut: assumes a run whose job was skipped concludes "skipped", not "success"; confirm on the first real runs.
+        run: |
+          last="$(gh run list --repo "$GITHUB_REPOSITORY" --workflow deploy.yml --status success --limit 1 --json headSha --jq '.[0].headSha // ""')"
+          if [[ -z "$last" ]] || ! git diff --quiet "$last" HEAD -- migrations/; then
+            echo "::error::migrations/ changed since the last deployed revision (${last:-none}). Back up, then deploy this release with Actions > deploy > Run workflow."
+            exit 1
+          fi
+      # Short-lived credentials through Workload Identity Federation; no service-account key exists.
+      - uses: google-github-actions/auth@7c6bc770dae815cd3e89ee6cdf493a5fab2cc093 # v3
+        with:
+          workload_identity_provider: ${{ vars.GCP_WORKLOAD_IDENTITY_PROVIDER }}
+          service_account: ${{ vars.GCP_DEPLOY_SERVICE_ACCOUNT }}
+      - uses: google-github-actions/setup-gcloud@aa5489c8933f4cc7a4f7d45035b3b1440c9c10db # v3.0.1
+      - name: Build the reviewed linux/amd64 image archive
+        run: ./scripts/release-image.sh
+      - name: Preflight against the VM (changes nothing)
+        run: ./scripts/deploy-vm.sh --preflight "dist/opportunity-app-${REVIEWED_SHA}-amd64.tar.gz"
+      - name: Deploy with recovery point, health checks and rollback
+        run: ./scripts/deploy-vm.sh "dist/opportunity-app-${REVIEWED_SHA}-amd64.tar.gz"
+      - name: Report the running release
+        if: always()
+        run: ./scripts/deploy-vm.sh --status
diff --git a/.github/workflows/hygiene.yml b/.github/workflows/hygiene.yml
index ba89a9c..41e2ec6 100644
--- a/.github/workflows/hygiene.yml
+++ b/.github/workflows/hygiene.yml
@@ -102,6 +102,57 @@ jobs:
             "select count(*) from pg_database where datname ~ '^opportunity_(auth|app|test)_[0-9a-f]{32}$'")"
           [[ "$remaining" == 0 ]] || { echo "Integration tests left temporary databases behind" >&2; exit 1; }
 
+  acceptance:
+    name: Product acceptance journey
+    runs-on: ubuntu-24.04
+    timeout-minutes: 30
+    services:
+      postgres:
+        image: postgres:16-alpine
+        ports:
+          - 5432:5432
+        env:
+          POSTGRES_USER: opportunity_ci
+          POSTGRES_PASSWORD: synthetic_ci_only
+          POSTGRES_DB: opportunity_ci
+        options: >-
+          --health-cmd "pg_isready -U opportunity_ci -d opportunity_ci"
+          --health-interval 10s
+          --health-timeout 5s
+          --health-retries 5
+    steps:
+      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
+        with:
+          persist-credentials: false
+      - uses: Swatinem/rust-cache@d1527c8f67ef9af357290475d4af4b0fab8f12e0 # v2.9.3
+        with:
+          cache-bin: false
+          save-if: false
+      - uses: actions/setup-node@949feb2413d6458794dcd2491c4babbbce0c15c1 # v7.1.0
+        with:
+          node-version: '22'
+          cache: npm
+          cache-dependency-path: |
+            apps/web/package-lock.json
+            apps/browser-worker/package-lock.json
+      - name: Build server, interface and browser worker
+        run: |
+          cargo build --locked --bin opportunity
+          npm ci --prefix apps/web && npm --prefix apps/web run build
+          npm ci --prefix apps/browser-worker && npm --prefix apps/browser-worker run build
+      - name: Install Chromium and its OS dependencies
+        run: npx --no-install playwright install --with-deps --only-shell chromium
+        working-directory: apps/browser-worker
+      # Synthetic CV, ranking corpus, mock ATS and mail fixture on loopback; every seed source disabled.
+      - name: Landing, login, CV, ranking, questions, mock submission, email, restart
+        env:
+          TEST_OFFLINE: '1'
+          TEST_DB_CONTAINER: ${{ job.services.postgres.id }}
+          TEST_DB_USER: opportunity_ci
+          TEST_DB_PASSWORD: synthetic_ci_only
+          TEST_OUTPUT_DIR: ${{ runner.temp }}/product-test
+        run: node scripts/test-product.mjs
+
   frontend:
     name: Frontend validation
     runs-on: ubuntu-24.04
diff --git a/Dockerfile b/Dockerfile
index f5f7d33..376896b 100644
--- a/Dockerfile
+++ b/Dockerfile
@@ -41,8 +41,8 @@ FROM debian:bookworm-slim AS runtime
 ARG GIT_REVISION
 LABEL org.opencontainers.image.revision=$GIT_REVISION \
       org.opencontainers.image.title="opportunity-runtime"
-# ca-certificates is needed to read job boards over TLS; curl is the health check.
-RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
+# ca-certificates is needed to read job boards over TLS; curl is the health check; poppler-utils provides pdftotext for PDF CVs.
+RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl poppler-utils \
     && rm -rf /var/lib/apt/lists/*
 # An unprivileged user owns the private tree, which holds the CV and browser artefacts.
 RUN useradd --system --create-home --uid 10001 opportunity
@@ -66,7 +66,7 @@ CMD ["serve"]
 #
 # The browser is installed by the same Playwright package that will drive it, rather than taken from a prebuilt browser image. Playwright pins an exact browser build per release, so a base image tagged for one version and a lockfile resolving to another produce an image with a browser in it and no browser it can use: `Executable doesn't exist at /ms-playwright/chromium_headless_shell-1248`. Installing from the resolved package makes that mismatch impossible, and brings only Chromium instead of Chromium, Firefox and WebKit.
 FROM node:22-bookworm-slim AS browser
-RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl \
+RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl poppler-utils \
     && rm -rf /var/lib/apt/lists/*
 RUN useradd --system --create-home --uid 10001 opportunity
 WORKDIR /app
diff --git a/README.md b/README.md
index 5e291bb..67b0196 100644
--- a/README.md
+++ b/README.md
@@ -8,4 +8,4 @@ Automatic applications require an explicit employer/profile authorization in the
 
 For email notifications, configure `MAIL_API_KEY`, `MAIL_FROM`, and `MAIL_TO` in the ignored `.env`. The default delivery API is Resend; `MAIL_API_URL` can point to a compatible transport. Acceptance is recorded separately from provider-confirmed delivery. No notification credentials are included.
 
-Focused browser/API verification: `node scripts/test-product.mjs`. It uses an isolated local PostgreSQL database, a synthetic CV, real public listings and loopback-only application/mail fixtures. Real applications and real recipient emails are never sent by this test.
+Product acceptance journey: `TEST_OFFLINE=1 node scripts/test-product.mjs` (run in CI). It uses an isolated PostgreSQL database, a synthetic CV and ranking corpus, and loopback-only application/mail fixtures, with every source disabled. Without `TEST_OFFLINE` it also waits for live discovery from real public listings, as a smoke test. Real applications and real recipient emails are never sent. Ranking rules: [docs/RANKING_POLICY.md](docs/RANKING_POLICY.md).
diff --git a/apps/web/public/linkedin-vibe.webp b/apps/web/public/linkedin-vibe.webp
new file mode 100644
index 0000000..db3d2c4
Binary files /dev/null and b/apps/web/public/linkedin-vibe.webp differ
diff --git a/apps/web/src/App.tsx b/apps/web/src/App.tsx
index 0d42da5..685c078 100644
--- a/apps/web/src/App.tsx
+++ b/apps/web/src/App.tsx
@@ -1,5 +1,6 @@
 import { useCallback, useEffect, useRef, useState } from "react";
 import { api, ApiError, type Home, type Question } from "./api";
+import LandingPage from "./components/LandingPage";
 import LoginPage from "./components/LoginPage";
 
 const labels: Record<string, string> = { saved: "Queued", preparing: "Checking form", ready: "Ready · authorization needed", needs_review: "Needs input", submitting: "Submitting", submitted: "Submitted", submission_unknown: "Unconfirmed · check with employer", failed: "Failed", unsupported: "Manual completion", interview: "Interview", rejected: "Rejected", withdrawn: "Withdrawn" };
@@ -14,40 +15,60 @@ function QuestionForm({ question, onAnswer }: { question: Question; onAnswer: (a
   </form>;
 }
 
+type Route = "landing" | "login" | "workspace";
+const routeOf = (path: string): Route => path === "/login" ? "login" : path === "/" || path === "/index.html" ? "landing" : "workspace";
+
 export default function App() {
-  const [session, setSession] = useState<"checking" | boolean>("checking");
+  const [route, setRoute] = useState<Route>(() => routeOf(window.location.pathname));
+  // null when nobody is signed in; `via` says whether the identity came from a password or the Cloudflare layer.
+  const [session, setSession] = useState<"checking" | null | { via: string }>("checking");
+  const [expired, setExpired] = useState(false);
+  const [upstream, setUpstream] = useState("");
   const [home, setHome] = useState<Home | null>(null);
   const [error, setError] = useState("");
   const [busy, setBusy] = useState("");
   const [notice, setNotice] = useState("");
   const input = useRef<HTMLInputElement>(null);
   const generation = useRef(0);
+  const go = useCallback((path: string, replace = false) => {
+    if (window.location.pathname !== path) window.history[replace ? "replaceState" : "pushState"]({}, "", path);
+    setRoute(routeOf(path));
+  }, []);
+  useEffect(() => { const onPop = () => setRoute(routeOf(window.location.pathname)); window.addEventListener("popstate", onPop); return () => window.removeEventListener("popstate", onPop); }, []);
+  // The server decides every time: the bundle is public, so nothing the browser believes about a session counts.
   const check = useCallback(async () => {
-    try { await api.session(); setSession(true); window.history.replaceState({}, "", "/"); }
-    catch (e) { setSession(false); if (!(e instanceof ApiError && e.status === 401)) setError("The application is unavailable. Try again shortly."); }
+    try { setSession(await api.session()); setExpired(false); }
+    catch (e) { setSession(null); if (e instanceof ApiError && e.status === 401) setUpstream(e.needs && e.needs !== "native" ? e.message : ""); else setError("The application is unavailable. Try again shortly."); }
   }, []);
+  const lost = useCallback(() => { setSession(null); setHome(null); setExpired(true); }, []);
   const reload = useCallback(async () => {
     const current = ++generation.current;
     try { const data = await api.home(); if (current === generation.current) setHome(data); }
-    catch (e) { if (e instanceof ApiError && e.status === 401) { setSession(false); setHome(null); } else setError(e instanceof Error ? e.message : "Could not load opportunities."); }
-  }, []);
+    catch (e) { if (e instanceof ApiError && e.status === 401) lost(); else setError(e instanceof Error ? e.message : "Could not load opportunities."); }
+  }, [lost]);
   useEffect(() => { void check(); }, [check]);
-  useEffect(() => { if (session !== true) return; void reload(); const timer = window.setInterval(() => void reload(), 15000); return () => clearInterval(timer); }, [session, reload]);
+  const native = session !== "checking" && session?.via === "password";
+  // A Cloudflare identity never had a front door at /, and a signed-in owner has nothing to do on /login.
+  useEffect(() => { if (session && session !== "checking" && (route === "login" || (route === "landing" && !native))) go("/workspace", true); }, [session, native, route, go]);
+  useEffect(() => { if (!session || session === "checking" || route !== "workspace") return; void reload(); const timer = window.setInterval(() => void reload(), 15000); return () => clearInterval(timer); }, [session, route, reload]);
   const act = async (key: string, action: () => Promise<unknown>, message = "") => {
     setBusy(key); setError(""); setNotice("");
     try { await action(); await reload(); setNotice(message); }
-    catch (e) { if (e instanceof ApiError && e.status === 401) setSession(false); setError(e instanceof Error ? e.message : "Please try again."); }
+    catch (e) { if (e instanceof ApiError && e.status === 401) lost(); setError(e instanceof Error ? e.message : "Please try again."); }
     finally { setBusy(""); }
   };
+  // The public page asks the server for nothing private and renders before the session check finishes.
+  if (route === "landing" && (session === "checking" || !session || native)) return <LandingPage signedIn={native} onLogin={() => go(native ? "/workspace" : "/login")} />;
   if (session === "checking") return <main className="loading" role="status">Opening Opportunities…</main>;
-  if (!session) return <><LoginPage onSignedIn={() => void check()} />{error && <p className="login-error" role="alert">{error}</p>}</>;
+  if (!session && upstream) return <main className="login" id="main"><div className="login-card"><h1>Alp&apos;s Opportunity Platform</h1><p className="notice error" role="alert">{upstream}</p><p className="login-sub">This deployment checks identity before the application. Reload to go through it again.</p><button type="button" className="primary" onClick={() => window.location.reload()}>Reload</button><p className="login-foot"><a href="https://alpblkba.dev/">← back to alpblkba.dev</a></p></div></main>;
+  if (!session) return <><LoginPage expired={expired} onSignedIn={() => void check()} />{error && <p className="login-error" role="alert">{error}</p>}</>;
   if (!home) return <main className="loading"><p role="status">Loading your opportunities…</p>{error && <p role="alert">{error}</p>}</main>;
   const applied = home.applications.filter(a => a.status === "submitted").length;
   const needing = new Set(home.questions.map(q => q.application_id)).size;
   const activePolicies = home.policies.filter(p => p.enabled && new Date(p.expires_at) > new Date());
   const profileName = home.profiles.find(p => p.id === home.selected_profile)?.name;
   return <main className="shell" id="main">
-    <header className="topbar"><span className="brand"><span className="brand-mark">O</span> Opportunity Platform</span><button className="quiet" onClick={() => void act("logout", async () => { await api.logout(); setSession(false); setHome(null); })}>Sign out</button></header>
+    <header className="topbar"><span className="brand"><span className="brand-mark">O</span> Opportunity Platform</span><button className="quiet" onClick={() => void api.logout().catch(() => undefined).then(() => { setSession(null); setHome(null); setExpired(false); go("/"); })}>Sign out</button></header>
     <section className="workspace" aria-labelledby="heading">
       <div className="heading-row"><div><h1 id="heading">Opportunities</h1><p className="muted">Your job search, automatically.</p></div><span className={`badge ${home.cv_ready ? "success" : "neutral"}`}>{home.cv_ready ? "CV uploaded" : "Add your CV"}</span></div>
       {error && <p className="message error" role="alert">{error}</p>}{notice && <p className="message" role="status">{notice}</p>}
@@ -58,13 +79,14 @@ export default function App() {
       <label className="muted profile-label" htmlFor="profile">Searching for</label>
       <select id="profile" value={home.selected_profile} disabled={!!busy} onChange={e => void act("profile", () => api.select(e.target.value))}>{home.profiles.map(p => <option key={p.id} value={p.id}>{p.name}</option>)}</select>
       <div className="metrics" aria-label="Application overview"><div><strong>{home.cv_ready ? home.matches : "—"}</strong><span>Matches</span></div><div><strong>{applied}</strong><span>Applied</span></div><div><strong>{needing}</strong><span>Needs input</span></div></div>
+      {(activePolicies.length > 0 || home.submissions_paused) && <div className="authorizations"><div><span><span className="dot" />{home.submissions_paused ? "All submissions paused" : "Automatic applications on"}<small>{home.submissions_paused ? "Nothing is prepared or sent until you resume." : `At most ${home.max_automatic_per_day} automatic applications per day across all employers.`}</small></span><button disabled={!!busy} onClick={() => void act("pause", () => api.pause(!home.submissions_paused), home.submissions_paused ? "Submissions resumed." : "All submissions paused.")}>{home.submissions_paused ? "Resume" : "Pause all"}</button></div></div>}
       {activePolicies.length > 0 && <div className="authorizations">{activePolicies.map(p => <div key={p.id}><span><span className="dot" />Auto-apply: {p.name}<small>{home.profiles.find(x => x.id === p.profile_id)?.name} · Until {date(p.expires_at)}</small></span><button disabled={!!busy} onClick={() => void act(p.id, () => api.disable(p.id))}>Stop</button></div>)}</div>}
       {home.questions.length > 0 && <section className="questions"><h2>Needs your input <span className="count">{home.questions.length}</span></h2>{home.questions.map(q => { const app = home.applications.find(a => a.id === q.application_id); return <div key={q.id} className="question-card"><p className="muted small">{app?.employer} · {app?.title}</p>{q.field_type === "unsupported" ? <p className="small">{q.question_text} · <a href={app?.url} target="_blank" rel="noreferrer">Complete on employer website ↗</a></p> : <QuestionForm question={q} onAnswer={answer => act(q.id, () => api.answer(q.id, answer), "Answer saved. The application will resume automatically.")} />}</div>; })}</section>}
       <section aria-labelledby="ranked-heading"><div className="section-heading"><h2 id="ranked-heading">Ranked opportunities</h2><span className="muted small">{home.last_updated ? `Updated ${date(home.last_updated)}` : "Finding current opportunities…"}</span></div>
         {!home.cv_ready ? <div className="empty"><span className="empty-icon">↥</span><h3>{home.cv ? "Update your CV to calculate matches" : "Your next opportunity starts here"}</h3><p>Upload your CV to see opportunities for this profile.</p></div> : home.jobs.length === 0 ? <div className="empty"><h3>{home.discovered ? "No strong matches for this profile yet" : "Searching for opportunities"}</h3><p>{home.discovered ? "Try another career profile. New opportunities are checked automatically." : "Current employer listings will appear here automatically."}</p></div> : <div className="jobs">{home.jobs.map((job, i) => {
           const app = home.applications.find(a => a.opportunity_id === job.id);
           const policy = activePolicies.find(p => p.profile_id === home.selected_profile && p.source_keys.includes(job.source_key));
-          return <article className="job" key={job.id}><div className="rank">{String(i + 1).padStart(2, "0")}</div><div className="job-body"><div className="job-top"><span className="employer">{job.organization}</span><span className={`badge ${job.score >= 65 ? "success" : "neutral"}`}>{job.score >= 65 ? "Strong fit" : "Potential fit"}</span></div><h3><a href={job.apply_url} target="_blank" rel="noreferrer">{job.canonical_title} <span aria-hidden="true">↗</span></a></h3><p className="muted small">{[job.city, job.country_code, job.remote_raw].filter(Boolean).join(" · ") || "Location not specified"} · {job.provider} · Seen {date(job.last_seen_at)}</p><p className="match-note">{job.explanation}</p>{job.gaps.length > 0 && <p className="gaps">{job.gaps.join(" · ")}</p>}
+          return <article className="job" key={job.id}><div className="rank">{String(i + 1).padStart(2, "0")}</div><div className="job-body"><div className="job-top"><span className="employer">{job.organization}</span><span className={`badge ${job.eligibility !== "needs_review" && job.score >= 65 ? "success" : "neutral"}`}>{job.eligibility === "needs_review" ? "Needs review" : job.score >= 65 ? "Priority match" : "Potential match"}</span></div><h3><a href={job.apply_url} target="_blank" rel="noreferrer">{job.canonical_title} <span aria-hidden="true">↗</span></a></h3><p className="muted small">{[job.city, job.country_code, job.remote_raw].filter(Boolean).join(" · ") || "Location not specified"} · {job.provider} · Seen {date(job.last_seen_at)}</p><p className="match-note">{job.explanation}</p>{job.technical !== undefined && <p className="muted small">Priority {Math.round(job.score)} · technical fit {Math.round(job.technical)} · career stage {Math.round(job.career_priority)} · profile {Math.round(job.profile_match)}</p>}{job.gaps.length > 0 && <p className="gaps">{job.gaps.join(" · ")}</p>}
             <div className="job-actions">{app ? <span className={`status ${app.status === "submitted" ? "success-text" : ""}`}>{labels[app.status] || app.status}</span> : <button disabled={!!busy} onClick={() => void act(job.id, () => api.apply(job.id), "Application queued. Any required questions will appear here.")}>Prepare application</button>}<a className="source-link" href={job.apply_url} target="_blank" rel="noreferrer">Employer page ↗</a></div>
             {!policy && ["greenhouse", "lever", "ashby"].includes(job.provider) && app?.status !== "unsupported" && <details className="auto-policy"><summary>Enable automatic applications at {job.organization}</summary><p>Apply to strong matches for <strong>{profileName}</strong> at {job.organization}, using your current CV and your answers. Up to 3 per day, 30 total, for 30 days. Required questions pause the application.</p><button disabled={!!busy} onClick={() => void act(`authorize-${job.id}`, () => api.authorize(job.source_key, home.selected_profile), `Automatic applications enabled at ${job.organization}.`)}>Authorize {job.organization}</button></details>}
           </div></article>;
diff --git a/apps/web/src/__tests__/Home.test.tsx b/apps/web/src/__tests__/Home.test.tsx
index 45faaa8..609350a 100644
--- a/apps/web/src/__tests__/Home.test.tsx
+++ b/apps/web/src/__tests__/Home.test.tsx
@@ -2,8 +2,9 @@ import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/re
 import { afterEach, expect, it, vi } from "vitest";
 import App from "../App";
 const home = { cv:{id:"cv1",label:"stored-cv.pdf",created_at:"2026-10-25T08:00:00Z"},cv_ready:true,profiles:[{id:"firmware",name:"Firmware and platform systems"},{id:"memory",name:"Memory and operating systems"}],selected_profile:"memory",jobs:[],matches:0,discovered:10,applications:[],questions:[],policies:[],mail:[],email_configured:false,last_updated:null };
-afterEach(()=>{cleanup();vi.unstubAllGlobals();});
+afterEach(()=>{cleanup();vi.unstubAllGlobals();window.history.replaceState({},"","/");});
 it("opens login immediately and shows the stored CV after authentication",async()=>{
+ window.history.replaceState({},"","/login");
  let signed=false;
  const fetch=vi.fn(async(path:string)=>{
   if(path.endsWith('/login')){signed=true;return Response.json({});}
@@ -35,3 +36,17 @@ it("requires a deliberate employer authorization and displays exact questions",a
  await waitFor(()=>expect(fetch.mock.calls.some(c=>c[0]==='/api/home/answer/q1')).toBe(true));
  expect(fetch.mock.calls.some(c=>c[0]==='/api/home/authorize')).toBe(false);
 });
+it("labels the blended priority honestly and flags postings that need review",async()=>{
+ const job={id:'j2',canonical_title:'PhD position, memory systems',organization:'Example Lab',provider:'academic',source_key:'lab',apply_url:'https://example.test',last_seen_at:'2026-10-25T08:00:00Z',score:81.4,technical:70.2,profile_match:40,career_stage:'PhD position',career_priority:60,eligibility:'needs_review',explanation:'PhD position · Your CV: DRAM',gaps:['No funded vacancy confirmed']};
+ vi.stubGlobal('fetch',vi.fn(async(path:string)=>Response.json(path.endsWith('/home')?{...home,jobs:[job],matches:1}:{})));render(<App/>);
+ expect(await screen.findByText('Needs review')).toBeDefined();
+ expect(screen.getByText('Priority 81 · technical fit 70 · career stage 60 · profile 40')).toBeDefined();
+ expect(screen.getByText('No funded vacancy confirmed')).toBeDefined();
+ expect(screen.queryByText('Strong fit')).toBeNull();
+});
+it("pauses and resumes every submission from one control",async()=>{
+ const fetch=vi.fn(async(path:string)=>Response.json(path.endsWith('/home')?{...home,submissions_paused:true,max_automatic_per_day:10}:{}));vi.stubGlobal('fetch',fetch);render(<App/>);
+ expect(await screen.findByText('All submissions paused')).toBeDefined();
+ fireEvent.click(screen.getByRole('button',{name:'Resume'}));
+ await waitFor(()=>expect(fetch.mock.calls.some(c=>c[0]==='/api/home/pause'&&JSON.parse(String((c as unknown as [string,RequestInit])[1]?.body)).paused===false)).toBe(true));
+});
diff --git a/apps/web/src/__tests__/Landing.test.tsx b/apps/web/src/__tests__/Landing.test.tsx
new file mode 100644
index 0000000..3fb5ca9
--- /dev/null
+++ b/apps/web/src/__tests__/Landing.test.tsx
@@ -0,0 +1,122 @@
+import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
+import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
+
+import App from "../App";
+
+const home = { cv: null, cv_ready: false, profiles: [{ id: "memory", name: "Memory and operating systems" }], selected_profile: "memory", jobs: [], matches: 0, discovered: 0, applications: [], questions: [], policies: [], mail: [], email_configured: false, last_updated: null };
+const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
+
+/** A server that answers the session check with `session` (null = nobody), and serves the workspace. */
+function server(session: null | { via: string }, calls: string[] = [], needs = "native") {
+  let signedIn = session;
+  return vi.fn(async (input: RequestInfo | URL) => {
+    const url = typeof input === "string" ? input : input.toString();
+    calls.push(url);
+    if (url === "/api/auth/login") { signedIn = { via: "password" }; return json({}); }
+    if (url === "/api/auth/logout") { signedIn = null; return json({}); }
+    if (!signedIn) return json({ error: "unauthorized", needs, detail: needs === "native" ? "no session" : "no access assertion" }, 401);
+    if (url === "/api/auth/session") return json({ email: "owner@example.test", display_name: "Alp", ...signedIn });
+    if (url === "/api/home") return json(home);
+    return json({});
+  });
+}
+
+beforeEach(() => window.history.replaceState({}, "", "/"));
+afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
+
+describe("the public landing page", () => {
+  it("says exactly what it is meant to say", () => {
+    vi.stubGlobal("fetch", server(null));
+    render(<App />);
+    expect(screen.getByRole("heading", { name: "Alp's Opportunity Platform" })).toBeDefined();
+    expect(screen.getByText("A personal platform for discovering, evaluating and tracking the things worth applying to.")).toBeDefined();
+    // The capitalisation is the joke, so it is the thing to assert.
+    expect(screen.getByText("bro casually replacing Linkedin")).toBeDefined();
+    expect(screen.queryByText(/LinkedIn/)).toBeNull();
+    expect(screen.getByRole("link", { name: "what is this?" }).getAttribute("href")).toBe("https://alpblkba.dev/projects/opportunity-platform/");
+    expect(screen.getByRole("link", { name: "back to home" }).getAttribute("href")).toBe("https://alpblkba.dev/");
+    expect(screen.getByRole("link", { name: "Log in as another user" }).getAttribute("href")).toBe("https://www.youtube.com/watch?v=dQw4w9WgXcQ");
+  });
+
+  it("carries the supplied meme, undistorted and described", () => {
+    vi.stubGlobal("fetch", server(null));
+    render(<App />);
+    const meme = screen.getByRole("img");
+    expect(meme.getAttribute("src")).toBe("/linkedin-vibe.webp");
+    expect(meme.getAttribute("width")).toBe("461");
+    expect(meme.getAttribute("height")).toBe("565");
+    expect((meme.getAttribute("alt") ?? "").length).toBeGreaterThan(30);
+  });
+
+  it("asks the server for nothing private, even for a signed-in owner", async () => {
+    const calls: string[] = [];
+    vi.stubGlobal("fetch", server({ via: "password" }, calls));
+    render(<App />);
+    await waitFor(() => expect(screen.getByRole("link", { name: "Log in as Alp (alpblkba)" }).getAttribute("href")).toBe("/workspace"));
+    expect(window.location.pathname).toBe("/");
+    expect(calls.every((url) => url === "/api/auth/session")).toBe(true);
+  });
+
+  it("goes to the login form, signs in to /workspace, and back returns to the landing", async () => {
+    vi.stubGlobal("fetch", server(null));
+    render(<App />);
+    fireEvent.click(screen.getByRole("link", { name: "Log in as Alp (alpblkba)" }));
+    fireEvent.change(await screen.findByLabelText("Email"), { target: { value: "owner@example.test" } });
+    expect(window.location.pathname).toBe("/login");
+    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "synthetic-password" } });
+    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
+    await screen.findByLabelText("Searching for");
+    expect(window.location.pathname).toBe("/workspace");
+
+    fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
+    await screen.findByText("bro casually replacing Linkedin");
+    expect(window.location.pathname).toBe("/");
+    fireEvent.click(screen.getByRole("link", { name: "Log in as Alp (alpblkba)" }));
+    await screen.findByLabelText("Password");
+    // Signing out is not a session that ran out.
+    expect(screen.queryByText(/session ran out/)).toBeNull();
+    window.history.back();
+    await screen.findByText("bro casually replacing Linkedin");
+  });
+
+  it("takes a Cloudflare-authenticated visitor straight to the workspace", async () => {
+    vi.stubGlobal("fetch", server({ via: "cloudflare-access" }));
+    render(<App />);
+    await screen.findByLabelText("Searching for");
+    expect(window.location.pathname).toBe("/workspace");
+    expect(screen.queryByText("bro casually replacing Linkedin")).toBeNull();
+  });
+});
+
+describe("the workspace boundary", () => {
+  it("shows the login form for a deep link with no session, and stays on the link after sign-in", async () => {
+    window.history.replaceState({}, "", "/workspace");
+    const calls: string[] = [];
+    vi.stubGlobal("fetch", server(null, calls));
+    render(<App />);
+    fireEvent.change(await screen.findByLabelText("Email"), { target: { value: "owner@example.test" } });
+    expect(screen.queryByLabelText("Searching for")).toBeNull();
+    expect(calls.includes("/api/home")).toBe(false);
+    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "synthetic-password" } });
+    fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
+    await screen.findByLabelText("Searching for");
+    expect(window.location.pathname).toBe("/workspace");
+  });
+
+  it("offers a reload, not a dead-end password form, when the identity layer in front is missing", async () => {
+    window.history.replaceState({}, "", "/workspace");
+    vi.stubGlobal("fetch", server(null, [], "identity"));
+    render(<App />);
+    expect(await screen.findByRole("button", { name: "Reload" })).toBeDefined();
+    expect(screen.queryByLabelText("Password")).toBeNull();
+    expect(screen.getByRole("link", { name: /back to alpblkba\.dev/ }).getAttribute("href")).toBe("https://alpblkba.dev/");
+  });
+
+  it("sends the login page's back link to the personal site", async () => {
+    window.history.replaceState({}, "", "/login");
+    vi.stubGlobal("fetch", server(null));
+    render(<App />);
+    const back = await screen.findByRole("link", { name: /back to alpblkba\.dev/ });
+    expect(back.getAttribute("href")).toBe("https://alpblkba.dev/");
+  });
+});
diff --git a/apps/web/src/api.ts b/apps/web/src/api.ts
index dee1944..5389503 100644
--- a/apps/web/src/api.ts
+++ b/apps/web/src/api.ts
@@ -11,7 +11,7 @@ export async function request<T>(path: string, init?: RequestInit): Promise<T> {
 }
 const post = <T,>(path: string, body = {}) => request<T>(path, { method: "POST", body: JSON.stringify(body) });
 export const api = {
-  session: () => request<{ email: string; display_name: string }>("/auth/session"),
+  session: () => request<{ email: string; display_name: string; via: string }>("/auth/session"),
   login: (email: string, password: string) => post("/auth/login", { email, password }),
   logout: () => post("/auth/logout"),
   home: () => request<Home>("/home"),
@@ -20,13 +20,14 @@ export const api = {
   disable: (id: string) => post(`/policies/${id}/state`, { enabled: false, revoke: true }),
   apply: (id: string) => post(`/home/apply/${id}`),
   answer: (id: string, answer: string) => post(`/home/answer/${id}`, { answer }),
+  pause: (paused: boolean) => post("/home/pause", { paused }),
   upload: (file: File) => request("/documents/cv?filename=" + encodeURIComponent(file.name), {
     method: "POST", headers: { "content-type": "application/octet-stream" }, body: file,
   }),
 };
 export interface Application { id: string; opportunity_id: string; status: string; reason?: string; title: string; employer: string; url: string }
 export interface Question { id: string; application_id: string; question_text: string; field_type: string; options: string[] }
-export interface Job { id: string; canonical_title: string; organization: string; city?: string; country_code?: string; remote_raw?: string; apply_url: string; last_seen_at: string; score: number; explanation: string; gaps: string[]; source_key: string; provider: string }
+export interface Job { id: string; canonical_title: string; organization: string; city?: string; country_code?: string; remote_raw?: string; apply_url: string; last_seen_at: string; score: number; technical: number; profile_match: number; career_stage: string; career_priority: number; eligibility: "eligible" | "needs_review"; explanation: string; gaps: string[]; source_key: string; provider: string }
 export interface Home {
   cv: { id: string; label: string; created_at: string } | null;
   cv_ready: boolean;
@@ -40,5 +41,7 @@ export interface Home {
   policies: { id: string; name: string; enabled: boolean; source_keys: string[]; profile_id: string; expires_at: string }[];
   last_updated: string | null;
   email_configured: boolean;
+  submissions_paused: boolean;
+  max_automatic_per_day: number;
   mail: { application_id: string; state: string; error?: string }[];
 }
diff --git a/apps/web/src/components/LandingPage.tsx b/apps/web/src/components/LandingPage.tsx
new file mode 100644
index 0000000..4cea3d0
--- /dev/null
+++ b/apps/web/src/components/LandingPage.tsx
@@ -0,0 +1,67 @@
+/** The personal site, which this page belongs beside rather than inside. */
+const PERSONAL_SITE = "https://alpblkba.dev/";
+/** What this thing is, written up over there. */
+const ABOUT = "https://alpblkba.dev/projects/opportunity-platform/";
+/** The joke. There is exactly one other account and it is not yours. */
+const OTHER_USER = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";
+
+interface Props {
+  /** True when a native session already exists, so the button goes in rather than to the form. */
+  signedIn?: boolean;
+  /** Moves to the login page, or straight to the workspace, without reloading the application. */
+  onLogin: () => void;
+}
+
+export default function LandingPage({ signedIn = false, onLogin }: Props) {
+  const destination = signedIn ? "/workspace" : "/login";
+  return (
+    <main className="landing" id="main">
+      <nav className="landing-nav" aria-label="Elsewhere">
+        <a href={PERSONAL_SITE}>back to home</a>
+      </nav>
+
+      <div className="landing-middle">
+        <h1>Alp&apos;s Opportunity Platform</h1>
+        <p className="landing-description">
+          A personal platform for discovering, evaluating and tracking the things worth applying to.
+        </p>
+
+        <div className="landing-actions">
+          <a
+            className="landing-button primary"
+            href={destination}
+            onClick={(event) => {
+              // Ordinary same-tab navigation, handled in the application so the page does not
+              // reload. The modifier keys are left alone so a middle or command click still opens
+              // a new tab, which is what every other link on the web does.
+              if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.shiftKey || event.button !== 0) {
+                return;
+              }
+              event.preventDefault();
+              onLogin();
+            }}
+          >
+            Log in as Alp (alpblkba)
+          </a>
+          <a className="landing-button" href={OTHER_USER}>
+            Log in as another user
+          </a>
+        </div>
+      </div>
+
+      <div className="landing-foot">
+        <p className="landing-tagline">bro casually replacing Linkedin</p>
+        <p className="landing-about">
+          <a href={ABOUT}>what is this?</a>
+        </p>
+        <img
+          className="landing-meme"
+          src="/linkedin-vibe.webp"
+          width={461}
+          height={565}
+          alt="A man on a porch looking back over his shoulder, captioned: I bring a sort of “fuck linkedin” vibe to linkedin that linkedin don't really like."
+        />
+      </div>
+    </main>
+  );
+}
diff --git a/apps/web/src/styles.css b/apps/web/src/styles.css
index 096d149..3964e3c 100644
--- a/apps/web/src/styles.css
+++ b/apps/web/src/styles.css
@@ -1 +1,155 @@
 :root{font-family:Inter,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;color:#ededed;background:#111211;font-synthesis:none;font-weight:400;line-height:1.5;color-scheme:dark}*{box-sizing:border-box}body{margin:0}button,input,select{font:inherit}button,a,input,select,summary{-webkit-tap-highlight-color:transparent}button{cursor:pointer;border:1px solid #3e413e;border-radius:999px;padding:8px 17px;background:transparent;color:#d7dad7;font-size:13px;font-weight:500;white-space:nowrap}button:hover{border-color:#737973;background:#232723}button:disabled{opacity:.5;cursor:wait}a{color:inherit;text-decoration:none}a:hover{text-decoration:underline}input,select{background:#181a18;border:1px solid #505650;border-radius:10px;color:#f1f2f1;padding:12px 14px;width:100%;min-width:0}select{cursor:pointer}input:focus,select:focus,button:focus-visible,a:focus-visible,summary:focus-visible{outline:2px solid #9cdb9f;outline-offset:3px}h1,h2,h3,p{margin:0}h1{font-size:29px;line-height:1.2;letter-spacing:-.8px;font-weight:600}h2{font-size:16px;font-weight:600;letter-spacing:-.15px}h3{font-size:17px;line-height:1.4;font-weight:550;letter-spacing:-.25px}.shell{max-width:1000px;margin:auto;padding:32px 30px 80px}.topbar{display:flex;align-items:center;justify-content:space-between;margin:0 0 34px;color:#b5bbb5}.brand{display:flex;gap:10px;align-items:center;font-size:13px;letter-spacing:.2px}.brand-mark{display:grid;place-items:center;width:27px;height:27px;background:#222b23;border:1px solid #456648;border-radius:8px;color:#ade5b1;font-size:17px}.quiet{border:0;color:#919991;padding:8px}.workspace{background:#191b19;border:1px solid #303430;border-radius:18px;padding:30px 30px 0;box-shadow:0 20px 80px #0002}.heading-row,.cv-row{display:flex;align-items:center;justify-content:space-between;gap:20px}.heading-row{margin-bottom:28px}.heading-row p{margin-top:6px}.muted{color:#9ba29b}.small{font-size:12px}.cv-name{font-size:14px;font-weight:550}.cv-row p{margin-top:3px}.profile-label{display:block;font-size:12px;margin:24px 0 8px}.file-input{position:absolute;width:1px;height:1px;opacity:0;padding:0;pointer-events:none}.badge{display:inline-block;font-size:11px;line-height:1.4;padding:4px 9px;border-radius:20px;white-space:nowrap}.success{color:#9bdd9f;background:#253b28}.neutral{color:#bec8bc;background:#2b302b}.primary{color:#122015;background:#b4deb0;border-color:#b4deb0}.primary:hover{background:#c7ecc3}.metrics{display:grid;grid-template-columns:repeat(3,1fr);border-top:1px solid #2c312c;margin-top:24px;padding:26px 0}.metrics strong{display:block;font-size:30px;font-weight:500;letter-spacing:-1px;font-variant-numeric:tabular-nums}.metrics span{font-size:12px;color:#9ba29b}.section-heading{display:flex;gap:15px;align-items:center;justify-content:space-between;padding:18px 0;border-top:1px solid #2c312c}.jobs{border-top:1px solid #2c312c}.job{display:flex;gap:17px;padding:24px 0;border-bottom:1px solid #2c312c}.rank{font-variant-numeric:tabular-nums;color:#626f63;font-size:12px;padding-top:2px}.job-body{flex:1;min-width:0}.job-top{display:flex;align-items:center;justify-content:space-between;gap:12px}.employer{color:#a8b8a7;font-size:12px;font-weight:500;letter-spacing:.1px}.job h3{margin:5px 0 7px}.job h3 span{color:#748474;font-size:13px}.match-note{font-size:13px;color:#d4ded2;margin-top:12px}.gaps{font-size:12px;color:#c7b58a;margin-top:6px}.job-actions{display:flex;align-items:center;gap:18px;margin-top:18px;font-size:12px}.job-actions button{padding:6px 13px;font-size:12px}.source-link{font-size:12px;color:#9ba29b}.auto-policy{font-size:12px;margin-top:14px;color:#99a698}.auto-policy summary{cursor:pointer;display:list-item;width:fit-content}.auto-policy p{margin:12px 0;max-width:620px;color:#bac5b9}.auto-policy button{font-size:12px}.status{font-size:12px;background:#282e28;padding:5px 10px;border-radius:5px}.success-text{color:#a6dea7}.empty{text-align:center;padding:38px 20px 48px;background:#141614;border-radius:10px}.empty p{margin-top:8px;color:#8d998e;font-size:13px}.empty-icon{display:block;font-size:30px;color:#9bb69b;margin-bottom:14px}.workspace-footer{display:flex;align-items:center;gap:7px;font-size:11px;color:#819081;padding:20px 0;flex-wrap:wrap}.dot{display:inline-block;background:#82b784;width:5px;height:5px;border-radius:50%;flex-shrink:0}.mail-status{margin-left:auto;color:#a89f87}.message{padding:11px 14px;font-size:13px;background:#233426;color:#c2e7c0;border-radius:8px;margin-bottom:18px}.error{background:#422a28;color:#f6c2b9}.questions{margin:8px 0 26px}.count{font-size:11px;background:#3d3526;color:#d4bf8f;border-radius:15px;padding:3px 8px;margin-left:8px}.question-card{padding:16px;background:#22271f;border:1px solid #3e4837;border-radius:10px;margin-top:12px}.question label{display:block;font-size:14px;margin:8px 0}.answer-row{display:flex;gap:10px}.answer-row input,.answer-row select{padding:8px 12px;font-size:13px}.authorizations{margin:0 0 24px;background:#202b21;border-radius:10px;padding:12px 16px;font-size:12px}.authorizations>div{display:flex;gap:15px;justify-content:space-between;align-items:center}.authorizations>div+div{margin-top:12px}.authorizations small{display:block;color:#9ba99a;margin-top:3px}.authorizations .dot{margin-right:7px}.authorizations button{font-size:11px;padding:5px 12px}.application-list{margin-top:28px}.application{display:flex;justify-content:space-between;gap:20px;padding:16px 0;border-bottom:1px solid #2c312c;font-size:13px}.application strong{font-weight:500}.application-state{text-align:right;flex-shrink:0;max-width:42%;font-size:12px}.application-state small,.application-state a{display:block;margin-top:5px}.loading{max-width:650px;margin:15vh auto;padding:30px;color:#aab4aa}.login{min-height:100vh;display:grid;place-items:center;padding:24px}.login-card{width:100%;max-width:410px;padding:36px;background:#191b19;border:1px solid #303430;border-radius:18px}.login-card h1{font-size:26px}.login-sub{font-size:13px;color:#9ba29b;margin:8px 0 28px}.login-card label{font-size:12px;display:block;margin:18px 0 7px}.login-card button{width:100%;margin-top:24px;border-radius:9px;padding:11px}.login-error{position:fixed;bottom:30px;text-align:center;width:100%;color:#edbcb5}.notice{font-size:13px;padding:10px;border-radius:8px}.login-prompt,.login-foot{display:none}@media(max-width:650px){.shell{padding:18px 14px 35px}.topbar{margin-bottom:20px}.workspace{padding:22px 18px 0;border-radius:14px}.heading-row{gap:12px}h1{font-size:25px}.heading-row .muted{font-size:13px}.job{gap:10px}.section-heading{align-items:flex-start;flex-direction:column;gap:4px}.job-top{align-items:flex-start}.cv-row{gap:12px}.cv-name{overflow-wrap:anywhere}.metrics strong{font-size:27px}.rank{display:none}.answer-row{flex-direction:column}.application{flex-direction:column;gap:8px}.application-state{text-align:left;max-width:none}.mail-status{margin-left:0;width:100%;padding-top:4px}.workspace-footer{font-size:10px}}
+
+
+/* Original landing palette (3d3ef77), used only by the public landing page below. */
+:root {
+  --bg: #0f1115;
+  --panel: #171a21;
+  --panel-raised: #1e222b;
+  --border: #2a303c;
+  --text: #e6e9ef;
+  --muted: #9aa4b5;
+  --accent: #6ea8fe;
+  --ok: #4ade80;
+  --warn: #fbbf24;
+  --bad: #f87171;
+  --radius: 10px;
+}
+
+@media (prefers-color-scheme: light) {
+  :root {
+    --bg: #f6f7f9;
+    --panel: #ffffff;
+    --panel-raised: #f0f2f6;
+    --border: #d7dbe3;
+    --text: #16181d;
+    --muted: #5b6474;
+    --accent: #1a56db;
+    --ok: #15803d;
+    --warn: #b45309;
+    --bad: #b91c1c;
+  }
+}
+
+:root:has(.landing) {
+  background: var(--bg);
+  color: var(--text);
+}
+
+/* The public landing. Three bands: a corner link, the thing itself in the middle, and the joke at
+   the bottom. Everything that is centred is centred because it is the page's one message; the
+   tagline and the link under it are left-aligned because they are an aside, not a headline. */
+.landing {
+  min-height: 100dvh;
+  display: grid;
+  grid-template-rows: auto auto 1fr;
+  gap: 2rem;
+  max-width: 44rem;
+  margin: 0 auto;
+  padding: 1.25rem 1.25rem 3rem;
+}
+
+.landing-nav {
+  display: flex;
+  justify-content: flex-end;
+  font-size: 0.9rem;
+}
+
+.landing-nav a {
+  color: var(--muted);
+}
+
+/* The page's one message is centred; the aside at the bottom is not. Near the top rather than
+   vertically centred, so the tagline and the meme below it are on the same screen. */
+.landing-middle {
+  align-self: start;
+  text-align: center;
+  display: grid;
+  gap: 1rem;
+  justify-items: center;
+}
+
+.landing-middle h1 {
+  margin: 0;
+  font-size: clamp(1.6rem, 5vw, 2.4rem);
+  line-height: 1.15;
+}
+
+.landing-description {
+  margin: 0;
+  max-width: 34rem;
+  color: var(--muted);
+  font-size: 1rem;
+  line-height: 1.5;
+}
+
+.landing-actions {
+  margin-top: 0.75rem;
+  display: grid;
+  gap: 0.6rem;
+  /* One width for both, so neither reads as the lesser option by accident. */
+  width: min(22rem, 100%);
+  justify-self: center;
+}
+
+a.landing-button {
+  display: block;
+  padding: 0.7rem 1rem;
+  border: 1px solid var(--border);
+  border-radius: 10px;
+  background: var(--panel);
+  color: var(--text);
+  text-align: center;
+  text-decoration: none;
+  font: inherit;
+  min-height: 44px;
+}
+
+a.landing-button:hover {
+  border-color: var(--accent);
+}
+
+a.landing-button.primary {
+  background: var(--accent);
+  border-color: var(--accent);
+  color: #0b0d12;
+  font-weight: 600;
+}
+
+a.landing-button:focus-visible {
+  outline: 2px solid var(--accent);
+  outline-offset: 2px;
+}
+
+.landing-foot {
+  display: grid;
+  gap: 0.35rem;
+  justify-items: start;
+}
+
+.landing-tagline {
+  margin: 0;
+  font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
+  color: var(--muted);
+}
+
+.landing-about {
+  margin: 0 0 0.75rem;
+  font-size: 0.9rem;
+}
+
+.landing-meme {
+  /* The one thing still centred on this page, because it is a picture rather than a line of text. */
+  justify-self: center;
+  width: min(380px, 100%);
+  height: auto;
+  border-radius: 10px;
+  border: 1px solid var(--border);
+}
+
+@media (max-width: 720px) {
+  .landing {
+    gap: 1.5rem;
+    padding-bottom: 2rem;
+  }
+}
diff --git a/crates/api/src/product.rs b/crates/api/src/product.rs
index 3866499..0904410 100644
--- a/crates/api/src/product.rs
+++ b/crates/api/src/product.rs
@@ -16,6 +16,7 @@ pub fn router() -> Router<AppState> {
         .route("/home/authorize", post(authorize))
         .route("/home/apply/{id}", post(apply))
         .route("/home/answer/{id}", post(answer))
+        .route("/home/pause", post(pause))
 }
 async fn home(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
     Ok(Json(product::dashboard(&s.db).await?))
@@ -47,6 +48,17 @@ async fn apply(State(s): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<V
     Ok(Json(json!({"id":product::enqueue(&s.db,id).await?})))
 }
 #[derive(Deserialize)]
+struct PauseBody {
+    paused: bool,
+}
+async fn pause(
+    State(s): State<AppState>,
+    Json(b): Json<PauseBody>,
+) -> Result<Json<Value>, ApiError> {
+    product::pause_submissions(&s.db, b.paused).await?;
+    Ok(Json(json!({"ok":true})))
+}
+#[derive(Deserialize)]
 struct AnswerBody {
     answer: String,
 }
diff --git a/crates/api/tests/native_auth.rs b/crates/api/tests/native_auth.rs
index d391b27..96e2702 100644
--- a/crates/api/tests/native_auth.rs
+++ b/crates/api/tests/native_auth.rs
@@ -954,8 +954,9 @@ async fn cv_upload_download_and_versions_work_with_native_and_cloudflare_identit
                 assertion("owner@example.test", chrono::Duration::hours(1)),
             )
         };
-        let url = format!("{}/api/documents/cv?filename=synthetic-cv.pdf", server.base);
-        let bytes = b"%PDF-1.4\nSynthetic CV fixture, no personal data.\n%%EOF";
+        // Uploads must be readable CVs; real PDF extraction is covered by scripts/test-cv-pdf.mjs.
+        let url = format!("{}/api/documents/cv?filename=synthetic-cv.txt", server.base);
+        let bytes = b"Ada Example\nSkills\nSynthetic CV fixture with Linux, Rust and DRAM, no personal data.\n";
         assert_eq!(
             server
                 .client
@@ -1020,7 +1021,7 @@ async fn cv_upload_download_and_versions_work_with_native_and_cloudflare_identit
             assert!(response.headers()["content-disposition"]
                 .to_str()
                 .unwrap()
-                .contains(".pdf"));
+                .contains(".txt"));
             assert_eq!(response.bytes().await.unwrap().as_ref(), bytes);
             let path: String = sqlx::query_scalar("select file_path from documents where id=$1")
                 .bind(Uuid::parse_str(&id).unwrap())
diff --git a/crates/core/src/mail.rs b/crates/core/src/mail.rs
index 38812bc..2c06cdd 100644
--- a/crates/core/src/mail.rs
+++ b/crates/core/src/mail.rs
@@ -26,7 +26,7 @@ pub async fn dispatch(db: &Db) -> anyhow::Result<()> {
     sqlx::query("update notifications set delivery_state='failed',delivery_error='Delivery could not be confirmed; automatic retry stopped to prevent duplicate email.' where delivery_receipt is null and first_attempt_at<now()-interval '23 hours' and delivered_at is null")
         .execute(db).await?;
     let mut tx = db.begin().await?;
-    let rows:Vec<(Uuid,String,Value,Option<String>)>=sqlx::query_as("select n.id,n.kind,jsonb_build_object('employer',org.display_name,'position',o.canonical_title,'url',(select p.url from postings p where p.opportunity_id=o.id order by p.url limit 1),'time',n.created_at,'detail',n.payload),n.delivery_receipt from notifications n join applications a on a.id=n.application_id join opportunities o on o.id=a.opportunity_id join organizations org on org.id=o.organization_id where n.kind in ('submitted','action_required') and n.delivered_at is null and n.delivery_state not in ('bounced','failed') and (n.last_attempt_at is null or n.last_attempt_at<now()-interval '1 minute') order by n.created_at for update of n skip locked limit 5").fetch_all(&mut *tx).await?;
+    let rows:Vec<(Uuid,String,Value,Option<String>)>=sqlx::query_as("select n.id,n.kind,jsonb_build_object('employer',org.display_name,'position',o.canonical_title,'url',(select p.url from postings p where p.opportunity_id=o.id order by p.url limit 1),'time',n.created_at,'detail',n.payload),n.delivery_receipt from notifications n join applications a on a.id=n.application_id join opportunities o on o.id=a.opportunity_id join organizations org on org.id=o.organization_id where n.kind in ('submitted','action_required','submission_uncertain','failed','manual_required') and n.delivered_at is null and n.delivery_state not in ('bounced','failed') and (n.last_attempt_at is null or n.last_attempt_at<now()-interval '1 minute') order by n.created_at for update of n skip locked limit 5").fetch_all(&mut *tx).await?;
     // Persist the attempt before network I/O: a process crash must not reset the retry window.
     for (id, _, _, _) in &rows {
         sqlx::query("update notifications set first_attempt_at=coalesce(first_attempt_at,now()),last_attempt_at=now() where id=$1")
@@ -42,8 +42,8 @@ pub async fn dispatch(db: &Db) -> anyhow::Result<()> {
             }
             let employer=info["employer"].as_str().unwrap_or("");
             let position=info["position"].as_str().unwrap_or("");
-            let status=if kind=="submitted"{"Confirmed submitted"}else{"Your answer is needed"};
-            let body=json!({"from":from,"to":[to],"subject":format!("{status}: {position} at {employer}"),"text":format!("{status}\nEmployer: {employer}\nPosition: {position}\nApplication: {}\nTime: {}\n{}",info["url"].as_str().unwrap_or(""),info["time"].as_str().unwrap_or(""),if kind=="submitted"{"The employer confirmed receipt."}else{"Open Opportunities to answer the employer's questions."})});
+            let (status,hint)=wording(&kind);
+            let body=json!({"from":from,"to":[to],"subject":format!("{status}: {position} at {employer}"),"text":format!("{status}\nEmployer: {employer}\nPosition: {position}\nApplication: {}\nTime: {}\n{hint}",info["url"].as_str().unwrap_or(""),info["time"].as_str().unwrap_or(""))});
             let response=client.post(format!("{base}/emails")).bearer_auth(&key).header("Idempotency-Key",id.to_string()).json(&body).send().await?.error_for_status()?.json::<Value>().await?;
             let receipt=response["id"].as_str().ok_or_else(||anyhow::anyhow!("Email provider returned no message identifier"))?;
             // Acceptance is not delivery. A later status read must confirm delivered.
@@ -66,3 +66,32 @@ pub async fn dispatch(db: &Db) -> anyhow::Result<()> {
     }
     Ok(())
 }
+
+/// Subject status and next step per notification kind. Anything that needs the owner says what to do.
+fn wording(kind: &str) -> (&'static str, &'static str) {
+    match kind {
+        "submitted" => ("Confirmed submitted", "The employer confirmed receipt."),
+        "submission_uncertain" => ("Check with the employer", "The form may have been sent, but no confirmation was seen. It will not be retried; check with the employer before applying again."),
+        "failed" => ("Application failed", "Nothing was confirmed as sent. Open Opportunities to retry it."),
+        "manual_required" => ("Complete on the employer website", "Automation cannot finish this form. Open Opportunities for the employer link."),
+        _ => ("Your answer is needed", "Open Opportunities to answer the employer's questions."),
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    #[test]
+    fn every_kind_that_needs_the_owner_has_its_own_subject() {
+        let subjects: std::collections::HashSet<_> = [
+            "submitted",
+            "action_required",
+            "submission_uncertain",
+            "failed",
+            "manual_required",
+        ]
+        .iter()
+        .map(|k| super::wording(k).0)
+        .collect();
+        assert_eq!(subjects.len(), 5);
+    }
+}
diff --git a/crates/core/src/matching.rs b/crates/core/src/matching.rs
index 0cbb3e6..76df257 100644
--- a/crates/core/src/matching.rs
+++ b/crates/core/src/matching.rs
@@ -107,7 +107,7 @@ pub(crate) async fn shortlist(
     .await
 }
 
-fn to_candidate(row: &ScoredRow) -> Candidate {
+pub(crate) fn to_candidate(row: &ScoredRow) -> Candidate {
     Candidate {
         title: row.canonical_title.clone(),
         organization: row.organization.clone(),
diff --git a/crates/core/src/product.rs b/crates/core/src/product.rs
index 77a36ca..3443897 100644
--- a/crates/core/src/product.rs
+++ b/crates/core/src/product.rs
@@ -64,11 +64,21 @@ pub async fn select(db: &Db, id: Uuid) -> Result<(), AppError> {
     Ok(())
 }
 
+/// Names the ranking rules in API output, so a score can be traced to the policy that produced it.
+pub const RANKING_POLICY: &str = "career-priority-v2";
+
 #[derive(Debug, Serialize)]
 pub struct RankedJob {
     #[serde(flatten)]
     pub job: matching::ScoredRow,
+    /// Priority for this owner, blending the three components below. Not a hiring probability.
     pub score: f32,
+    pub technical: f32,
+    pub profile_match: f32,
+    pub career_stage: &'static str,
+    pub career_priority: f32,
+    /// "eligible", or "needs_review" when a gap below cannot be settled from the CV and posting.
+    pub eligibility: &'static str,
     pub explanation: String,
     pub gaps: Vec<String>,
     pub auto_eligible: bool,
@@ -76,9 +86,185 @@ pub struct RankedJob {
     pub provider: String,
 }
 
-fn direction(key: &str) -> &'static [&'static str] {
+/// Classified from the posting kind and title only: descriptions often mention supervising
+/// interns or a past internship without being internships themselves.
+#[derive(Debug, Clone, Copy, PartialEq, Eq)]
+enum CareerStage {
+    Thesis,
+    Internship,
+    WorkingStudent,
+    ResearchAssistant,
+    Phd,
+    NewGrad,
+    Junior,
+    Unspecified,
+    Mid,
+    Senior,
+    Staff,
+}
+
+impl CareerStage {
+    fn label(self) -> &'static str {
+        match self {
+            Self::Thesis => "Master's thesis",
+            Self::Internship => "Internship",
+            Self::WorkingStudent => "Working student",
+            Self::ResearchAssistant => "Research assistant",
+            Self::Phd => "PhD position",
+            Self::NewGrad => "New graduate",
+            Self::Junior => "Junior",
+            Self::Unspecified => "Level not stated",
+            Self::Mid => "Mid-level",
+            Self::Senior => "Senior",
+            Self::Staff => "Staff / principal",
+        }
+    }
+}
+
+/// Initial, uncalibrated policy: (technical, saved profile, career stage) weights.
+fn weights(profile_key: &str) -> (f32, f32, f32) {
+    if profile_key == "research" {
+        (0.45, 0.20, 0.35)
+    } else {
+        (0.55, 0.15, 0.30)
+    }
+}
+
+/// 0-100 preference for a stage. Research has its own table so a funded PhD vacancy is not
+/// outranked by an internship's stage bonus, while an unconfirmed PhD is never treated as funded.
+fn stage_priority(stage: CareerStage, profile_key: &str, advertised_state: &str) -> f32 {
+    use CareerStage::*;
+    let research = profile_key == "research";
+    match stage {
+        Thesis => 100.0,
+        Internship if research => 85.0,
+        Internship => 100.0,
+        WorkingStudent if research => 80.0,
+        WorkingStudent | ResearchAssistant => 92.0,
+        Phd if research && advertised_state == "advertised" => 95.0,
+        Phd if research => 60.0,
+        Phd => 65.0,
+        NewGrad if research => 72.0,
+        NewGrad => 78.0,
+        Junior if research => 68.0,
+        Junior => 74.0,
+        Unspecified => 52.0,
+        Mid => 36.0,
+        Senior => 8.0,
+        Staff => 2.0,
+    }
+}
+
+/// Recover an explicit title category only for generic feed classifications.
+fn effective_kind(job: &matching::ScoredRow) -> &str {
+    use opportunity_domain::OpportunityKind as K;
+    if matches!(job.kind.as_str(), "employment" | "other" | "part_time") {
+        match opportunity_domain::classify_kind(&job.canonical_title, None) {
+            K::MasterThesis => return "master_thesis",
+            K::Internship => return "internship",
+            K::WorkingStudent => return "working_student",
+            K::ResearchAssistant => return "research_assistant",
+            K::Phd => return "phd",
+            _ => {}
+        }
+    }
+    &job.kind
+}
+
+fn career_stage(job: &matching::ScoredRow) -> CareerStage {
+    use CareerStage::*;
+    match effective_kind(job) {
+        "master_thesis" => return Thesis,
+        "internship" => return Internship,
+        "working_student" => return WorkingStudent,
+        "research_assistant" => return ResearchAssistant,
+        "phd" => return Phd,
+        _ => {}
+    }
+    let title = opportunity_domain::normalize_location(Some(&job.canonical_title));
+    let has = |words: &[&str]| {
+        words
+            .iter()
+            .any(|w| opportunity_domain::normalize::contains_phrase(&title, w))
+    };
+    // German academic employment ("wissenschaftliche/r Mitarbeiter/in") is in practice a doctoral post.
+    if title.contains("wissenschaftlich") && title.contains("mitarbeiter") {
+        return Phd;
+    }
+    if has(&[
+        "staff",
+        "principal",
+        "distinguished",
+        "director",
+        "head of",
+        "vp",
+    ]) {
+        return Staff;
+    }
+    if has(&["senior", "sr", "lead", "manager", "iii", "l5", "l6"]) {
+        return Senior;
+    }
+    if has(&[
+        "mid",
+        "mid level",
+        "midlevel",
+        "mid career",
+        "intermediate",
+        "ii",
+        "l4",
+    ]) {
+        return Mid;
+    }
+    if has(&["student"]) {
+        return WorkingStudent;
+    }
+    if has(&[
+        "new grad",
+        "new graduate",
+        "new college graduate",
+        "graduate",
+        "early career",
+        "early talent",
+        "entry level",
+        "entrylevel",
+        "absolvent",
+        "berufseinsteiger",
+        "trainee",
+    ]) {
+        return NewGrad;
+    }
+    if has(&["junior", "jr", "associate engineer", "engineer i", "l3"]) {
+        return Junior;
+    }
+    // A plain "Software Engineer" is not assumed junior; a stated requirement only downranks.
+    match required_years(job.description_text.as_deref().unwrap_or("")) {
+        Some(years) if years >= 8 => Senior,
+        Some(years) if years >= 5 => Mid,
+        _ => Unspecified,
+    }
+}
+
+fn priority_score(technical: f32, profile: f32, stage: f32, profile_key: &str) -> f32 {
+    let (t, p, s) = weights(profile_key);
+    (t * technical.clamp(0.0, 100.0) + p * profile.clamp(0.0, 100.0) + s * stage).clamp(0.0, 100.0)
+}
+
+/// Neither stage priority nor a profile score grants permission to submit; the per-employer
+/// authorization is still checked separately before any submission.
+fn auto_application_eligible(
+    cv_auto_eligible: bool,
+    profile_fit: &opportunity_domain::MatchResult,
+    advertised_state: &str,
+) -> bool {
+    cv_auto_eligible
+        && profile_fit.eligible
+        && profile_fit.score >= 20.0
+        && advertised_state == "advertised"
+}
+
+fn direction(key: &str) -> Vec<&'static str> {
     match key {
-        "firmware" => &[
+        "firmware" => vec![
             "firmware",
             "embedded",
             "RTOS",
@@ -86,7 +272,7 @@ fn direction(key: &str) -> &'static [&'static str] {
             "device driver",
             "platform systems",
         ],
-        "memory" => &[
+        "memory" => vec![
             "memory",
             "DRAM",
             "CXL",
@@ -96,7 +282,7 @@ fn direction(key: &str) -> &'static [&'static str] {
             "Linux",
             "storage",
         ],
-        "architecture" => &[
+        "architecture" => vec![
             "CPU",
             "RTL",
             "microarchitecture",
@@ -106,7 +292,7 @@ fn direction(key: &str) -> &'static [&'static str] {
             "ASIC",
             "architecture",
         ],
-        "ai-systems" => &[
+        "ai-systems" => vec![
             "AI",
             "GPU",
             "accelerator",
@@ -116,14 +302,12 @@ fn direction(key: &str) -> &'static [&'static str] {
             "co-design",
             "inference",
         ],
-        _ => &[
-            "research",
-            "thesis",
-            "doctoral",
-            "PhD",
-            "Masterarbeit",
-            "internship",
-        ],
+        // Research positions in the same four technical areas. Stage words such as "internship"
+        // gave any internship technical credit; thesis/PhD kinds come from the profile's job kinds.
+        _ => ["firmware", "memory", "architecture", "ai-systems"]
+            .into_iter()
+            .flat_map(direction)
+            .collect(),
     }
 }
 
@@ -143,11 +327,12 @@ fn fit(
             &opportunity_domain::normalize_location(Some(term)),
         )
     };
-    let title_matches = direction(key)
+    let terms = direction(key);
+    let title_matches = terms
         .iter()
         .filter(|w| contains(&normalized_title, w))
         .count();
-    let body_matches = direction(key)
+    let body_matches = terms
         .iter()
         .filter(|w| contains(&normalized_description, w))
         .count();
@@ -273,24 +458,73 @@ pub async fn ranked(db: &Db, profile_id: Uuid) -> Result<Vec<RankedJob>, AppErro
         return Ok(vec![]);
     };
     let profile = profiles::get(db, profile_id).await?;
-    let rows = matching::shortlist(db, &matching::MatchQuery::default()).await?;
+    let key = profile.profile.key.as_str();
+    let rules = profiles::rules_for(db, Some(profile_id)).await?;
+    // Load only rows that contain a direction term, which fit() requires anyway, so the shortlist
+    // cap can no longer drop an older relevant posting behind newer irrelevant ones.
+    let terms = direction(key)
+        .iter()
+        .map(|t| {
+            if t.contains(' ') {
+                format!("\"{t}\"")
+            } else {
+                (*t).to_owned()
+            }
+        })
+        .collect::<Vec<_>>()
+        .join(" or ");
+    let query = matching::MatchQuery {
+        q: Some(terms),
+        ..Default::default()
+    };
+    let rows = matching::shortlist(db, &query).await?;
     let sources: Vec<(Uuid,String,String)>=sqlx::query_as("select distinct on(p.opportunity_id) p.opportunity_id,s.key,s.kind from postings p join sources s on s.id=p.source_id where p.closed_at is null and p.opportunity_id is not null order by p.opportunity_id,p.url")
         .fetch_all(db).await?;
     let mut ranked = Vec::new();
     for row in rows {
-        let (score, explanation, gaps, auto_eligible) = fit(&cv, &profile.profile.key, &row);
-        if score < 20.0 {
+        let (technical, evidence, mut gaps, cv_auto_eligible) = fit(&cv, key, &row);
+        // No stage bonus can rescue an unrelated technical match.
+        if technical < 20.0 {
+            continue;
+        }
+        let applicable: Vec<_> = rules
+            .iter()
+            .filter(|(_, target)| target.is_none() || *target == Some(row.id))
+            .map(|(rule, _)| rule.clone())
+            .collect();
+        let mut candidate = matching::to_candidate(&row);
+        candidate.kind = effective_kind(&row).to_owned();
+        let profile_fit =
+            opportunity_domain::evaluate(&profile.profile, &candidate, &applicable, None);
+        // Saved kinds, location preferences and blacklist rules are hard exclusions.
+        if !profile_fit.eligible {
             continue;
         }
         let Some((_, source_key, provider)) = sources.iter().find(|s| s.0 == row.id) else {
             continue;
         };
+        let stage = career_stage(&row);
+        if stage == CareerStage::Phd && row.advertised_state != "advertised" {
+            gaps.push("No funded vacancy confirmed".into());
+        }
+        let career_priority = stage_priority(stage, key, &row.advertised_state);
+        let auto_eligible =
+            auto_application_eligible(cv_auto_eligible, &profile_fit, &row.advertised_state);
         ranked.push(RankedJob {
-            job: row,
-            score,
-            explanation,
+            score: priority_score(technical, profile_fit.score, career_priority, key),
+            technical,
+            profile_match: profile_fit.score,
+            career_stage: stage.label(),
+            career_priority,
+            eligibility: if gaps.is_empty() {
+                "eligible"
+            } else {
+                "needs_review"
+            },
+            explanation: format!("{} · {evidence}", stage.label()),
             gaps,
             auto_eligible,
+            job: row,
             source_key: source_key.clone(),
             provider: provider.clone(),
         });
@@ -341,12 +575,21 @@ pub async fn dashboard(db: &Db) -> Result<Value, AppError> {
             .fetch_one(db)
             .await?;
     let questions = applications::list_review_tasks(db, None).await?;
+    let (paused, cap) = workflow::brakes(db).await?;
     let mail:Vec<Value>=sqlx::query_scalar("select jsonb_build_object('application_id',application_id,'state',delivery_state,'error',delivery_error) from notifications where kind='submitted' order by created_at desc limit 100").fetch_all(db).await?;
     Ok(
-        json!({"cv":cv,"profiles":profiles,"selected_profile":profile_id,"jobs":jobs,"matches":matches,"discovered":discovered,"applications":applications,"questions":questions,"policies":policies,"last_updated":last_updated,"mail":mail,"email_configured":crate::mail::configured(),"cv_ready":current_candidate(db).await?.is_some()}),
+        json!({"cv":cv,"profiles":profiles,"selected_profile":profile_id,"jobs":jobs,"matches":matches,"discovered":discovered,"applications":applications,"questions":questions,"policies":policies,"last_updated":last_updated,"mail":mail,"email_configured":crate::mail::configured(),"cv_ready":current_candidate(db).await?.is_some(),"ranking_policy":RANKING_POLICY,"submissions_paused":paused,"max_automatic_per_day":cap}),
     )
 }
 
+/// The owner's emergency stop: refuses every submission and halts automatic preparation.
+pub async fn pause_submissions(db: &Db, paused: bool) -> Result<(), AppError> {
+    let owner = profiles::local_owner(db).await?;
+    sqlx::query("insert into product_settings(owner_id,submissions_paused) values($1,$2) on conflict(owner_id) do update set submissions_paused=$2,updated_at=now()")
+        .bind(owner).bind(paused).execute(db).await?;
+    Ok(())
+}
+
 pub async fn authorize_source(db: &Db, source_key: &str, profile_id: Uuid) -> Result<(), AppError> {
     select(db, profile_id).await?;
     let source = crate::sources::get_by_key(db, source_key)
@@ -391,6 +634,25 @@ pub async fn enqueue(db: &Db, opportunity_id: Uuid) -> Result<Uuid, AppError> {
     Ok(app.id)
 }
 
+/// Hand an application to the owner: no automation will touch it again, so they are told once.
+async fn manual(db: &Db, id: Uuid, reason: &str) -> Result<(), AppError> {
+    applications::set_status(db, id, "unsupported", json!({})).await?;
+    sqlx::query("update applications set route_reason=$2 where id=$1")
+        .bind(id)
+        .bind(reason)
+        .execute(db)
+        .await?;
+    applications::notify(
+        db,
+        Some(id),
+        "manual_required",
+        &format!("manual:{id}"),
+        json!({"reason": reason}),
+    )
+    .await?;
+    Ok(())
+}
+
 pub async fn process_application(db: &Db, id: Uuid) -> Result<(), AppError> {
     let app = applications::get_application(db, id).await?;
     if !["saved", "ready"].contains(&app.status.as_str()) {
@@ -414,9 +676,7 @@ pub async fn process_application(db: &Db, id: Uuid) -> Result<(), AppError> {
         .await?;
     // Real browser forms are supported only for these providers; no login or CAPTCHA bypass.
     if !["greenhouse", "lever", "ashby"].contains(&provider.as_str()) {
-        applications::set_status(db, id, "unsupported", json!({})).await?;
-        sqlx::query("update applications set route_reason='Complete this application on the employer website.' where id=$1").bind(id).execute(db).await?;
-        return Ok(());
+        return manual(db, id, "Complete this application on the employer website.").await;
     }
     let prepared = match workflow::prepare(db, id, &url).await {
         Ok(p) => p,
@@ -424,27 +684,26 @@ pub async fn process_application(db: &Db, id: Uuid) -> Result<(), AppError> {
             if error.to_string().contains("source refresh") {
                 return Ok(());
             }
-            applications::set_status(db, id, "unsupported", json!({})).await?;
-            sqlx::query("update applications set route_reason=$2 where id=$1")
-                .bind(id)
-                .bind({
-                    tracing::warn!(%error, "form unavailable");
-                    "This employer form requires manual completion.".to_owned()
-                })
-                .execute(db)
-                .await?;
-            return Ok(());
+            tracing::warn!(%error, "form unavailable");
+            return manual(db, id, "This employer form requires manual completion.").await;
         }
     };
     if prepared.plan.has_captcha
         || prepared.plan.requires_login
         || prepared.plan.submit_selector.is_none()
         || prepared.plan.fields.is_empty()
-        || prepared.plan.fields.iter().any(|f| f.required && f.kind == opportunity_domain::FieldKind::Unsupported)
+        || prepared
+            .plan
+            .fields
+            .iter()
+            .any(|f| f.required && f.kind == opportunity_domain::FieldKind::Unsupported)
     {
-        applications::set_status(db, id, "unsupported", json!({})).await?;
-        sqlx::query("update applications set route_reason='The employer requires manual completion or a verification challenge.' where id=$1").bind(id).execute(db).await?;
-        return Ok(());
+        return manual(
+            db,
+            id,
+            "The employer requires manual completion or a verification challenge.",
+        )
+        .await;
     }
     if !prepared.mapping.open.is_empty() {
         return Ok(());
@@ -477,6 +736,10 @@ pub async fn tick(db: &Db) -> Result<(), AppError> {
     sqlx::query("update application_attempts set state='unknown',diagnostic='Application interrupted; receipt must be checked with the employer.',finished_at=now() where state in ('started','filled') and started_at<now()-interval '10 minutes'").execute(db).await?;
     sqlx::query("update applications set status='submission_unknown',updated_at=now() where status='submitting' and updated_at<now()-interval '10 minutes'").execute(db).await?;
     sqlx::query("update applications set status='saved',updated_at=now() where status='preparing' and updated_at<now()-interval '10 minutes' and not exists(select 1 from application_attempts t where t.application_id=applications.id and t.state in ('started','submitted','unknown'))").execute(db).await?;
+    if workflow::brakes(db).await?.0 {
+        lock.commit().await?;
+        return Ok(());
+    }
     let policy_profiles:Vec<Uuid>=sqlx::query_scalar("select distinct profile_id from authorization_policies where enabled and revoked_at is null and expires_at>now() and profile_id is not null").fetch_all(db).await?;
     for profile_id in policy_profiles {
         for job in ranked(db, profile_id)
@@ -528,4 +791,230 @@ mod tests {
         assert_eq!(required_years("Requires 7+ years of experience"), Some(7));
         assert_eq!(required_years("Our company was founded in 2001"), None);
     }
+
+    use opportunity_domain::{ExclusionKind, ExclusionRule, Profile};
+
+    fn row(title: &str, kind: &str, description: &str) -> matching::ScoredRow {
+        matching::ScoredRow {
+            id: Uuid::new_v4(),
+            canonical_title: title.to_owned(),
+            organization: "Example Silicon".into(),
+            kind: kind.into(),
+            advertised_state: "advertised".into(),
+            country_code: Some("DE".into()),
+            city: Some("Munich".into()),
+            remote_raw: None,
+            deadline_at: None,
+            supervisor: None,
+            first_seen_at: Utc::now(),
+            last_seen_at: Utc::now(),
+            title_only: description.is_empty(),
+            apply_url: None,
+            description_text: Some(description.to_owned()),
+            department: None,
+            source_names: vec!["Example Silicon".into()],
+        }
+    }
+
+    fn assess(profile: &Profile, job: &matching::ScoredRow, rules: &[ExclusionRule]) -> bool {
+        let mut candidate = matching::to_candidate(job);
+        candidate.kind = effective_kind(job).to_owned();
+        opportunity_domain::evaluate(profile, &candidate, rules, None).eligible
+    }
+
+    #[test]
+    fn titles_classify_into_career_stages() {
+        use CareerStage::*;
+        for (title, kind, expected) in [
+            ("Masterarbeit: RISC-V", "master_thesis", Thesis),
+            ("Masterarbeit: DRAM", "employment", Thesis),
+            ("Firmware Internship", "employment", Internship),
+            ("Firmware Werkstudent (m/w/d)", "employment", WorkingStudent),
+            (
+                "Student Researcher, Memory Systems",
+                "employment",
+                WorkingStudent,
+            ),
+            ("PhD Candidate in Memory Systems", "employment", Phd),
+            (
+                "Doctoral Researcher, Computer Architecture",
+                "employment",
+                Phd,
+            ),
+            (
+                "Wissenschaftliche/r Mitarbeiter/in Rechnerarchitektur",
+                "employment",
+                Phd,
+            ),
+            (
+                "New College Graduate, GPU Architecture",
+                "employment",
+                NewGrad,
+            ),
+            ("Early Talent Firmware Engineer", "employment", NewGrad),
+            ("Junior Firmware Engineer", "employment", Junior),
+            ("Firmware Engineer I", "employment", Junior),
+            ("Associate Engineer, RTL", "employment", Junior),
+            ("Firmware Engineer", "employment", Unspecified),
+            ("Research Engineer, Accelerators", "employment", Unspecified),
+            ("Firmware Engineer II", "employment", Mid),
+            ("Mid-Level Firmware Engineer", "employment", Mid),
+            ("Senior Firmware Engineer", "employment", Senior),
+            ("Sr. RTL Design Engineer", "employment", Senior),
+            ("Principal CPU Architect", "employment", Staff),
+            ("Staff Engineer, Memory", "employment", Staff),
+        ] {
+            assert_eq!(career_stage(&row(title, kind, "")), expected, "{title}");
+        }
+    }
+
+    #[test]
+    fn body_text_never_changes_the_stage_except_to_downrank() {
+        use CareerStage::*;
+        let stage = |title, body| career_stage(&row(title, "employment", body));
+        assert_eq!(
+            stage("Software Engineer", "You will mentor interns."),
+            Unspecified
+        );
+        assert_eq!(stage("International Firmware Engineer", ""), Unspecified);
+        assert_eq!(
+            stage("Firmware Engineer", "Requires 6+ years of experience"),
+            Mid
+        );
+        assert_eq!(
+            stage("Firmware Engineer", "Requires 9+ years of experience"),
+            Senior
+        );
+    }
+
+    #[test]
+    fn equal_relevance_orders_thesis_and_internship_then_student_junior_mid_senior() {
+        use CareerStage::*;
+        let score = |stage| {
+            priority_score(
+                75.0,
+                60.0,
+                stage_priority(stage, "memory", "advertised"),
+                "memory",
+            )
+        };
+        let order = [
+            Thesis,
+            WorkingStudent,
+            NewGrad,
+            Junior,
+            Unspecified,
+            Mid,
+            Senior,
+            Staff,
+        ];
+        assert_eq!(score(Thesis), score(Internship));
+        for pair in order.windows(2) {
+            assert!(
+                score(pair[0]) > score(pair[1]),
+                "{:?} > {:?}",
+                pair[0],
+                pair[1]
+            );
+        }
+        // Strong technical fit beats a weak internship; the score stays bounded.
+        assert!(
+            priority_score(
+                95.0,
+                75.0,
+                stage_priority(Junior, "firmware", "advertised"),
+                "firmware"
+            ) > priority_score(
+                35.0,
+                40.0,
+                stage_priority(Internship, "firmware", "advertised"),
+                "firmware"
+            )
+        );
+        assert!(priority_score(150.0, 150.0, 100.0, "firmware") <= 100.0);
+    }
+
+    #[test]
+    fn research_keeps_a_funded_phd_ahead_of_an_internship_but_not_an_unfunded_one() {
+        use CareerStage::*;
+        let research = |stage, state| {
+            priority_score(
+                70.0,
+                60.0,
+                stage_priority(stage, "research", state),
+                "research",
+            )
+        };
+        assert!(research(Phd, "advertised") > research(Internship, "advertised"));
+        assert!(research(Phd, "open_topic") < research(Internship, "advertised"));
+        assert!(research(Thesis, "open_topic") >= research(Phd, "advertised"));
+        // Outside research the engineering table applies.
+        assert!(
+            stage_priority(Internship, "firmware", "advertised")
+                > stage_priority(Phd, "firmware", "advertised")
+        );
+    }
+
+    #[test]
+    fn an_unrelated_internship_gets_no_technical_credit_in_any_profile() {
+        let cv = CandidateProfile {
+            skills: vec!["Linux".into(), "DRAM".into()],
+            ..Default::default()
+        };
+        let marketing = row(
+            "Marketing Internship",
+            "internship",
+            "Social media and brand strategy",
+        );
+        for (key, _) in TRACKS {
+            assert_eq!(fit(&cv, key, &marketing).0, 0.0, "{key}");
+        }
+        let thesis = row(
+            "Masterarbeit: DRAM reliability",
+            "master_thesis",
+            "DRAM memory Linux",
+        );
+        assert!(fit(&cv, "research", &thesis).0 >= 20.0);
+    }
+
+    #[test]
+    fn saved_location_kind_and_blacklist_exclude_before_ranking() {
+        let mut profile = Profile::blank("memory", "Memory systems");
+        profile.terms = vec!["DRAM".into()];
+        profile.preferred_countries = vec!["DE".into()];
+        profile.job_kinds = vec!["internship".into(), "master_thesis".into()];
+        let mut job = row("DRAM Internship", "internship", "DRAM memory systems");
+        assert!(assess(&profile, &job, &[]));
+        job.country_code = Some("FR".into());
+        assert!(!assess(&profile, &job, &[]));
+        job.country_code = Some("DE".into());
+        job.kind = "employment".into();
+        job.canonical_title = "DRAM Engineer".into();
+        assert!(!assess(&profile, &job, &[]));
+        job.kind = "internship".into();
+        let rule = ExclusionRule {
+            kind: ExclusionKind::Company,
+            value: "Example Silicon".into(),
+            note: None,
+        };
+        assert!(!assess(&profile, &job, &[rule]));
+    }
+
+    #[test]
+    fn stage_priority_never_authorizes_an_automatic_application() {
+        let mut profile = Profile::blank("memory", "Memory systems");
+        profile.terms = vec!["DRAM".into()];
+        let job = row("DRAM Internship", "internship", "DRAM memory systems");
+        let mut candidate = matching::to_candidate(&job);
+        candidate.kind = effective_kind(&job).to_owned();
+        let fit = opportunity_domain::evaluate(&profile, &candidate, &[], None);
+        assert!(fit.eligible);
+        assert!(!auto_application_eligible(false, &fit, "advertised"));
+        assert!(!auto_application_eligible(true, &fit, "open_topic"));
+        assert!(!auto_application_eligible(
+            true,
+            &fit,
+            "potential_supervisor"
+        ));
+    }
 }
diff --git a/crates/core/src/profiles.rs b/crates/core/src/profiles.rs
index 5f9b8a4..3e7570e 100644
--- a/crates/core/src/profiles.rs
+++ b/crates/core/src/profiles.rs
@@ -379,7 +379,7 @@ type DefaultProfile = (
     &'static [&'static str],
 );
 
-/// The four tracks the contract asks for. Deliberately generic: these are technical starting points, not anyone's personal preferences, and they carry no location, authorization or experience data.
+/// The five career tracks the product offers. Deliberately generic: these are technical starting points, not anyone's personal preferences, and they carry no location, authorization or experience data.
 fn default_profiles() -> Vec<DefaultProfile> {
     vec![
         (
diff --git a/crates/core/src/workflow.rs b/crates/core/src/workflow.rs
index f3bdb5c..6980e90 100644
--- a/crates/core/src/workflow.rs
+++ b/crates/core/src/workflow.rs
@@ -233,6 +233,53 @@ pub struct SubmitReport {
 /// Submit an application, if and only if an authorization permits it right now.
 ///
 /// The sequence matters and is the reason this is one function: the verdict is taken from current state, the attempt is claimed under a unique idempotency key in the same breath as spending the authorization, and only then does the browser open. A second request for the same payload finds the existing attempt and does nothing.
+/// (paused, daily cap on automatic submissions), with defaults before the owner saves any setting.
+pub async fn brakes(pool: &Db) -> Result<(bool, i32), AppError> {
+    Ok(sqlx::query_as(
+        "select coalesce(bool_or(submissions_paused), false), coalesce(min(max_automatic_per_day), 10) from product_settings",
+    )
+    .fetch_one(pool)
+    .await?)
+}
+
+/// Owner-wide brakes above every employer policy: a pause stops all submissions, and a daily cap
+/// limits automatic (policy) ones however many employers are authorized.
+pub async fn submission_brake(pool: &Db, automatic: bool) -> Result<Option<String>, AppError> {
+    let (paused, cap) = brakes(pool).await?;
+    if paused {
+        return Ok(Some("All submissions are paused by the owner.".into()));
+    }
+    if automatic {
+        let today: i64 = sqlx::query_scalar("select count(*) from application_attempts where authorization_kind='policy' and started_at > now() - interval '1 day'")
+            .fetch_one(pool)
+            .await?;
+        if today >= i64::from(cap) {
+            return Ok(Some(format!(
+                "The daily limit of {cap} automatic applications is reached."
+            )));
+        }
+    }
+    Ok(None)
+}
+
+async fn refuse_if_braked(
+    pool: &Db,
+    application_id: Uuid,
+    automatic: bool,
+) -> Result<(), AppError> {
+    let Some(reason) = submission_brake(pool, automatic).await? else {
+        return Ok(());
+    };
+    applications::record_event(
+        pool,
+        application_id,
+        "submission_refused",
+        serde_json::json!({ "reason": reason }),
+    )
+    .await?;
+    Err(AppError::Invalid(reason))
+}
+
 pub async fn submit(
     pool: &Db,
     application_id: Uuid,
@@ -240,6 +287,7 @@ pub async fn submit(
     source_key: &str,
     document_path: Option<String>,
 ) -> Result<SubmitReport, AppError> {
+    refuse_if_braked(pool, application_id, false).await?;
     // Held across the inspection, the authorization and the submission itself, so a sweep cannot start underneath a form that is halfway filled.
     let slot = claim_browser_slot(pool).await?;
     let report =
@@ -314,6 +362,8 @@ async fn submit_holding_the_machine(
         opportunity_domain::Authorization::Policy { .. } => "policy",
         _ => "token",
     };
+    // Inside the browser slot, so two automatic submissions cannot both pass the daily cap.
+    refuse_if_braked(pool, application_id, kind == "policy").await?;
     let (attempt, newly) =
         applications::begin_attempt(pool, application_id, url, &hash, kind, policy_id, token_id)
             .await?;
diff --git a/crates/core/tests/applications.rs b/crates/core/tests/applications.rs
index ec51550..b74a81c 100644
--- a/crates/core/tests/applications.rs
+++ b/crates/core/tests/applications.rs
@@ -543,24 +543,53 @@ async fn missing_or_revoked_authorization_prevents_submission() {
 async fn product_policy_is_bound_to_the_discovered_destination_and_profile() {
     let db = require_db!();
     profiles::seed_default_profiles(&db.pool).await.unwrap();
-    let profile = profiles::get_by_key(&db.pool, "memory").await.unwrap().unwrap();
+    let profile = profiles::get_by_key(&db.pool, "memory")
+        .await
+        .unwrap()
+        .unwrap();
     let opportunity = seed_opportunity(&db.pool, "Memory engineer").await;
-    let app = applications::create_application(&db.pool, opportunity, Some(profile.id)).await.unwrap();
+    let app = applications::create_application(&db.pool, opportunity, Some(profile.id))
+        .await
+        .unwrap();
     let policy = enabled_policy(&db.pool, "example-greenhouse").await;
     sqlx::query("update authorization_policies set profile_id=$2 where id=$1")
-        .bind(policy).bind(profile.id).execute(&db.pool).await.unwrap();
+        .bind(policy)
+        .bind(profile.id)
+        .execute(&db.pool)
+        .await
+        .unwrap();
     let mut ctx = context(app.id, "example-greenhouse", "payload", Route::Auto).await;
-    assert!(!applications::decide_authorization(&db.pool, &ctx).await.unwrap().0.is_permitted());
+    assert!(!applications::decide_authorization(&db.pool, &ctx)
+        .await
+        .unwrap()
+        .0
+        .is_permitted());
     let source: Uuid = sqlx::query_scalar("insert into sources(key,display_name,kind,endpoint) values('example-greenhouse','Example','greenhouse','{\"kind\":\"greenhouse\",\"board_token\":\"example\"}') returning id")
         .fetch_one(&db.pool).await.unwrap();
     sqlx::query("insert into postings(source_id,native_id,opportunity_id,url,title,content_hash) values($1,'example',$2,$3,'Memory engineer','example')")
         .bind(source).bind(opportunity).bind(ctx.destination).execute(&db.pool).await.unwrap();
-    assert!(applications::decide_authorization(&db.pool, &ctx).await.unwrap().0.is_permitted());
+    assert!(applications::decide_authorization(&db.pool, &ctx)
+        .await
+        .unwrap()
+        .0
+        .is_permitted());
     ctx.destination = "https://job-boards.greenhouse.io/another-employer/jobs/123";
-    assert!(!applications::decide_authorization(&db.pool, &ctx).await.unwrap().0.is_permitted());
+    assert!(!applications::decide_authorization(&db.pool, &ctx)
+        .await
+        .unwrap()
+        .0
+        .is_permitted());
     ctx.destination = "http://127.0.0.1:8099/apply/single";
-    sqlx::query("update applications set profile_id=null where id=$1").bind(app.id).execute(&db.pool).await.unwrap();
-    assert!(!applications::decide_authorization(&db.pool, &ctx).await.unwrap().0.is_permitted());
+    sqlx::query("update applications set profile_id=null where id=$1")
+        .bind(app.id)
+        .execute(&db.pool)
+        .await
+        .unwrap();
+    assert!(!applications::decide_authorization(&db.pool, &ctx)
+        .await
+        .unwrap()
+        .0
+        .is_permitted());
     db.cleanup().await;
 }
 
@@ -1015,7 +1044,7 @@ async fn profiles_and_discovery_from_earlier_sessions_still_work_alongside_appli
     let db = require_db!();
     // Session 3 behaviour must survive the session 4 migration.
     let created = profiles::seed_default_profiles(&db.pool).await.unwrap();
-    assert_eq!(created.len(), 4);
+    assert_eq!(created.len(), 5);
     let mut profile = Profile::blank("custom", "Custom");
     profile.terms = vec!["DRAM".into()];
     let stored = profiles::create(&db.pool, profile).await.unwrap();
@@ -1027,7 +1056,7 @@ async fn profiles_and_discovery_from_earlier_sessions_still_work_alongside_appli
             .terms,
         vec!["DRAM"]
     );
-    assert_eq!(profiles::list(&db.pool).await.unwrap().len(), 5);
+    assert_eq!(profiles::list(&db.pool).await.unwrap().len(), 6);
     db.cleanup().await;
 }
 
@@ -1554,3 +1583,65 @@ async fn manual_tracking_persists_notes_and_owner_history_without_a_submission_a
         .is_err());
     db.cleanup().await;
 }
+
+#[tokio::test]
+async fn the_owner_pause_and_daily_cap_refuse_submissions_above_every_policy() {
+    let db = require_db!();
+    let application = application_ready_to_submit(&db.pool, "Firmware Engineer").await;
+    assert!(workflow::submission_brake(&db.pool, true)
+        .await
+        .unwrap()
+        .is_none());
+
+    opportunity_core::product::pause_submissions(&db.pool, true)
+        .await
+        .unwrap();
+    // Refused before the browser slot: no worker exists in this test and no attempt is begun.
+    let error = workflow::submit(
+        &db.pool,
+        application,
+        "http://127.0.0.1:8099/apply/single",
+        "mock",
+        None,
+    )
+    .await
+    .unwrap_err();
+    assert!(error.to_string().contains("paused"), "{error}");
+    let attempts: i64 = sqlx::query_scalar("select count(*) from application_attempts")
+        .fetch_one(&db.pool)
+        .await
+        .unwrap();
+    assert_eq!(attempts, 0);
+    let refused: i64 = sqlx::query_scalar(
+        "select count(*) from application_events where application_id=$1 and kind='submission_refused'",
+    )
+    .bind(application)
+    .fetch_one(&db.pool)
+    .await
+    .unwrap();
+    assert_eq!(refused, 1);
+
+    // The cap counts real automatic attempts across all employers; owner-approved one-offs are not automatic.
+    opportunity_core::product::pause_submissions(&db.pool, false)
+        .await
+        .unwrap();
+    sqlx::query("update product_settings set max_automatic_per_day=1")
+        .execute(&db.pool)
+        .await
+        .unwrap();
+    sqlx::query("insert into application_attempts (application_id,idempotency_key,destination,payload_hash,authorization_kind) values ($1,'cap-test','http://127.0.0.1:8099/apply/single','hash','policy')")
+        .bind(application)
+        .execute(&db.pool)
+        .await
+        .unwrap();
+    let reason = workflow::submission_brake(&db.pool, true)
+        .await
+        .unwrap()
+        .expect("the cap is reached");
+    assert!(reason.contains("daily limit of 1"), "{reason}");
+    assert!(workflow::submission_brake(&db.pool, false)
+        .await
+        .unwrap()
+        .is_none());
+    db.cleanup().await;
+}
diff --git a/crates/core/tests/discovery.rs b/crates/core/tests/discovery.rs
index 0b15fd2..312ae33 100644
--- a/crates/core/tests/discovery.rs
+++ b/crates/core/tests/discovery.rs
@@ -1695,13 +1695,13 @@ async fn a_profile_survives_a_restart_with_every_field_intact() {
 }
 
 #[tokio::test]
-async fn the_four_starting_profiles_seed_once_and_carry_no_personal_data() {
+async fn the_five_starting_profiles_seed_once_and_carry_no_personal_data() {
     let db = require_db!();
     let created = profiles::seed_default_profiles(&db.pool).await.unwrap();
     assert_eq!(
         created.len(),
-        4,
-        "the contract asks for four tracks: {created:?}"
+        5,
+        "the product has five career tracks: {created:?}"
     );
 
     let all = profiles::list(&db.pool).await.unwrap();
@@ -1742,7 +1742,7 @@ async fn the_four_starting_profiles_seed_once_and_carry_no_personal_data() {
         .await
         .unwrap()
         .is_empty());
-    assert_eq!(profiles::list(&db.pool).await.unwrap().len(), 4);
+    assert_eq!(profiles::list(&db.pool).await.unwrap().len(), 5);
     assert_eq!(
         profiles::get(&db.pool, all[0].id)
             .await
diff --git a/docs/DEPLOYMENT.md b/docs/DEPLOYMENT.md
index db0a531..d58eb95 100644
--- a/docs/DEPLOYMENT.md
+++ b/docs/DEPLOYMENT.md
@@ -158,6 +158,53 @@ Follow the owner's separately approved production rollout procedure. Build the e
 
 Migrations run before the server binds, in transactions recorded by SQLx. Migration 0005 preserves legacy rows, but the older runtime refuses its unknown migration history. Automatic image recovery cannot reverse that history. Recovery to the pre-0005 binary requires a new database restored from the pre-release backup and the matching retained deployment files/checker; it must preserve the failed database for investigation. Do not edit migration history or overwrite the live database to bypass this gate.
 
+## Automated deployment
+
+`.github/workflows/deploy.yml` runs after `continuous integration` succeeds on a push to `master`:
+it builds the reviewed archive with `release-image.sh`, then runs `deploy-vm.sh --preflight` and
+`deploy-vm.sh` over IAP, then `--status`. A release that changes `migrations/` since the last
+deployed revision is refused automatically; deploy it on purpose with **Actions → deploy → Run
+workflow** after checking the backup. Failed runs are emailed by GitHub. The workflow does nothing
+until `DEPLOY_ENABLED` is `true`.
+
+One-time bootstrap by the owner (no keys are created; GitHub receives short-lived tokens):
+
+```bash
+PROJECT=project-484f9dc3-1a36-4b55-9e8
+NUMBER="$(gcloud projects describe "$PROJECT" --format='value(projectNumber)')"
+gcloud iam workload-identity-pools create github --project="$PROJECT" --location=global
+gcloud iam workload-identity-pools providers create-oidc opportunity-platform --project="$PROJECT" \
+  --location=global --workload-identity-pool=github --issuer-uri=https://token.actions.githubusercontent.com \
+  --attribute-mapping=google.subject=assertion.sub,attribute.repository=assertion.repository,attribute.environment=assertion.environment \
+  --attribute-condition="assertion.repository=='alpblkba/opportunity-platform' && assertion.environment=='production'"
+gcloud iam service-accounts create opportunity-deploy --project="$PROJECT"
+SA="opportunity-deploy@${PROJECT}.iam.gserviceaccount.com"
+gcloud iam service-accounts add-iam-policy-binding "$SA" --project="$PROJECT" --role=roles/iam.workloadIdentityUser \
+  --member="principalSet://iam.googleapis.com/projects/${NUMBER}/locations/global/workloadIdentityPools/github/attribute.repository/alpblkba/opportunity-platform"
+for role in roles/compute.viewer roles/iap.tunnelResourceAccessor roles/compute.osAdminLogin; do
+  gcloud projects add-iam-policy-binding "$PROJECT" --member="serviceAccount:${SA}" --role="$role"
+done
+```
+
+If the VM runs as a service account, also grant `roles/iam.serviceAccountUser` on that account to
+`$SA`. OS Login must be enabled on the instance (`enable-oslogin=TRUE`); check with
+`gcloud compute instances describe opportunity-platform --zone us-central1-a --format='value(metadata.items)'`
+before changing it, because without OS Login `gcloud compute ssh` writes keys into instance metadata.
+
+Then in GitHub: create the `production` environment limited to the `master` branch, and set:
+
+```bash
+gh variable set GCP_PROJECT --body project-484f9dc3-1a36-4b55-9e8
+gh variable set GCP_INSTANCE --body opportunity-platform
+gh variable set GCP_ZONE --body us-central1-a
+gh variable set GCP_DEPLOY_SERVICE_ACCOUNT --body "opportunity-deploy@project-484f9dc3-1a36-4b55-9e8.iam.gserviceaccount.com"
+gh variable set GCP_WORKLOAD_IDENTITY_PROVIDER --body "projects/${NUMBER}/locations/global/workloadIdentityPools/github/providers/opportunity-platform"
+gh variable set DEPLOY_ENABLED --body true
+```
+
+The first workflow deployment must be a manual **Run workflow**, which establishes the deployed
+revision the schema check compares against.
+
 ## Known limitations
 
 Cloudflare Access assertion validation is tested against assertions signed with a test key held in the repository: the positive path accepts a correctly signed, correctly addressed, unexpired token for an allowlisted identity, and the negative paths refuse a wrong audience, a wrong issuer, an expired token, an unknown key id, a tampered payload, an identity outside the allowlist, a token with no identity, `alg: none` and garbage. The deployed instance has since been signed into with a real Access one-time code and returned authenticated API responses, so the team domain, the key endpoint and the audience tag are confirmed as well as the verification logic.
diff --git a/docs/RANKING_POLICY.md b/docs/RANKING_POLICY.md
new file mode 100644
index 0000000..d614ac8
--- /dev/null
+++ b/docs/RANKING_POLICY.md
@@ -0,0 +1,49 @@
+# Opportunity ranking (`career-priority-v2`)
+
+The dashboard's priority is a ranking heuristic for this owner, not a hiring probability or a
+qualification score. It is computed per request from the selected profile and the current CV
+(`crates/core/src/product.rs`).
+
+## Order of operations
+
+1. **Candidate set.** Only open opportunities whose search text contains one of the profile's
+   direction terms are loaded, so the `MATCH_SHORTLIST` cap never drops an older relevant posting
+   behind newer irrelevant ones.
+2. **Technical fit (0-100).** Direction terms in the title (or two in the body), CV skills found in
+   the posting, CV projects/employment, minus 10 per gap. Below 20 the posting is dropped; no stage
+   bonus can rescue it. The research profile uses the four technical areas' terms; stage words
+   ("internship", "thesis") give no technical credit.
+3. **Hard exclusions.** The saved profile's job kinds, location preferences and global or
+   per-profile blacklist rules drop the posting.
+4. **Review gaps.** Experience years, required PhD, work authorization wording, title-only
+   listings, and PhD posts without a confirmed vacancy mark the posting `needs_review`. Unknown is
+   never treated as eligible.
+5. **Priority** = `t * technical + p * profile + s * career_stage`, clamped to 0-100.
+
+| Profile | t | p | s |
+| --- | ---: | ---: | ---: |
+| firmware, memory, architecture, ai-systems | 0.55 | 0.15 | 0.30 |
+| research | 0.45 | 0.20 | 0.35 |
+
+| Career stage (from kind or title only) | Engineering | Research |
+| --- | ---: | ---: |
+| Master's thesis | 100 | 100 |
+| Internship | 100 | 85 |
+| Working student / student role | 92 | 80 |
+| Research assistant | 92 | 92 |
+| PhD, advertised vacancy | 65 | 95 |
+| PhD, topic or supervisor page only | 65 | 60 |
+| New graduate / early career | 78 | 72 |
+| Junior / Engineer I / associate | 74 | 68 |
+| Level not stated | 52 | 52 |
+| Mid / Engineer II / 5-7 years required | 36 | 36 |
+| Senior / lead / 8+ years required | 8 | 8 |
+| Staff / principal / director | 2 | 2 |
+
+These values are an initial policy, not calibrated against outcomes.
+
+## Automatic applications
+
+Ranking never authorizes a submission. Auto-queueing requires the unboosted CV match to be strong
+with no gaps, the saved-profile assessment to pass, an advertised vacancy, and a separate,
+unexpired employer authorization created by the owner.
diff --git a/docs/implementation/BLOCKERS.md b/docs/implementation/BLOCKERS.md
new file mode 100644
index 0000000..74df14d
--- /dev/null
+++ b/docs/implementation/BLOCKERS.md
@@ -0,0 +1,9 @@
+# Blockers (external, need the owner)
+
+| Blocker | Needed for | What the owner must provide |
+| --- | --- | --- |
+| GCP Workload Identity Federation pool/provider and deploy service account | automated CD | project id, provider resource name, service account email (repository variables) |
+| GitHub `production` environment with branch rule `master` | gated deploys | repository settings |
+| Off-host backup bucket and encryption key | PR-10 | bucket name, key handling choice |
+| Larger VM (2-4 GiB) for Chromium | unattended applications | approval of the monthly cost |
+| Real submission policy | live applications | the owner enabling an employer authorization in the app |
diff --git a/docs/implementation/DECISIONS.md b/docs/implementation/DECISIONS.md
new file mode 100644
index 0000000..0583a4a
--- /dev/null
+++ b/docs/implementation/DECISIONS.md
@@ -0,0 +1,21 @@
+# Decisions
+
+1. **Snapshot the uncommitted tree as its own commit** (`06fb29a`) on a feature branch, so every
+   later change is a reviewable diff and nothing the owner wrote can be lost.
+2. **Keep the stack** (Rust/Axum, React, PostgreSQL, Playwright, Compose, Actions, GCE VM,
+   Cloudflare). Nothing found justifies a change.
+3. **Keep the external-navigation guard** in the browser worker. Unattended real submissions stay a
+   deliberate owner switch (`BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised`), not a default.
+4. **Shortlist truncation is fixed by a SQL prefilter on the profile's direction terms**, not by
+   raising the limit or caching rankings: `fit()` already discards rows without those terms.
+5. **Experience years stay explicit-claim only.** Summing employment date ranges would let
+   telecom years clear an RTL "3+ years" gap; without domain attribution it is either wrong or
+   changes nothing. Revisit only with owner-confirmed per-domain experience facts.
+6. **Research profile uses the four technical areas as its direction terms.** Its old terms were
+   stage words, which gave any internship 40 technical points.
+7. **Ranking weights live in code (`product.rs`), versioned by `RANKING_POLICY`.** No runtime config
+   until someone needs to change them without a release.
+8. **Pause and daily cap sit in `workflow::submit`**, the one path every real submission takes; the
+   cap is checked inside the browser slot, which already serializes submissions.
+9. **No registry**: CD reuses the archive-over-IAP release path the scripts already verify.
+   Schema-changing releases are manual because an older image refuses a newer migration history.
diff --git a/docs/implementation/DEPLOYMENT_READINESS.md b/docs/implementation/DEPLOYMENT_READINESS.md
new file mode 100644
index 0000000..92388ec
--- /dev/null
+++ b/docs/implementation/DEPLOYMENT_READINESS.md
@@ -0,0 +1,12 @@
+# Deployment readiness
+
+| Question | Answer (2026-10-27) |
+| --- | --- |
+| Is automatic deployment implemented? | Yes, `.github/workflows/deploy.yml`. |
+| Is it connected to GitHub/GCP? | No. Needs the bootstrap in `docs/DEPLOYMENT.md` "Automated deployment" and `DEPLOY_ENABLED`. |
+| Has a real deployment been performed from this branch? | No. Nothing was pushed or deployed. |
+| Is the live application healthy? | Not checked: no read-only VM access from this machine yet (`gcloud` missing). |
+| Schema changes on this branch | `0006`, `0007` (owner WIP), `0008` (brakes, notification kinds). The first deploy must be a manual run after a verified backup; image rollback cannot undo them. |
+| Runtime image change | `poppler-utils` added so PDF CVs can be read in containers. |
+| Browser automation in production | Still off on e2-micro; external navigation still needs `BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised`. |
+| Backups | Scripted on the VM; off-host copy still manual (`BLOCKERS.md`). |
diff --git a/docs/implementation/HANDOFF.md b/docs/implementation/HANDOFF.md
new file mode 100644
index 0000000..e1a950b
--- /dev/null
+++ b/docs/implementation/HANDOFF.md
@@ -0,0 +1,34 @@
+# Handoff
+
+Read this, then `TASK_STATE.md`, then `git status` / `git log --oneline -15`.
+
+- Branch: `modernization/production-readiness` (local only, not pushed). Never commit to `master`;
+  never force-push. `master` == `origin/master` == `1043ab9`, untouched.
+- Commits: `06fb29a` owner WIP snapshot · `7f00fc2` baseline repairs · `10addad` tracker ·
+  `f0744e8` ranking v2 · `2588c32` landing · `ec27d2e` CI acceptance · `02371d7` pause/cap ·
+  `6f87dcb` failure emails · `b665d94` deploy workflow · then this tracker update.
+- Local test database: a disposable container, never the owner's `opportunity-db` on 5432.
+
+```bash
+docker run -d --rm --name opportunity-ci-scratch -e POSTGRES_USER=opportunity_ci \
+  -e POSTGRES_PASSWORD=synthetic_ci_only -e POSTGRES_DB=opportunity_ci -p 127.0.0.1:55432:5432 postgres:16-alpine
+export DATABASE_URL=postgres://opportunity_ci:synthetic_ci_only@127.0.0.1:55432/opportunity_ci
+cargo fmt --all -- --check && cargo clippy --locked --workspace --all-targets -- -D warnings && cargo test --locked --workspace
+(cd apps/web && npm ci && npm run typecheck && npm test && npm run build)
+bash scripts/verify-browser-worker.sh
+cargo build --locked --bin opportunity && TEST_OFFLINE=1 TEST_DB_CONTAINER=opportunity-ci-scratch \
+  TEST_DB_USER=opportunity_ci TEST_DB_PASSWORD=synthetic_ci_only TEST_DB_PORT=55432 node scripts/test-product.mjs
+```
+
+Do not repeat: the baseline snapshot commit; `npm ci` only if `node_modules` breaks.
+
+## Next actions
+
+1. Owner: push the branch and open a PR so CI (including the new acceptance job) runs on GitHub.
+2. Done: read-only IAP access works (`gcloud` 590.0.1 via Homebrew; prepend
+   `/opt/homebrew/share/google-cloud-sdk/bin` to PATH; the GCE key must be in ssh-agent, and its
+   passphrase is now in the macOS keychain). Any production change still needs the owner's go-ahead.
+3. Owner: GCP/GitHub bootstrap in `docs/DEPLOYMENT.md` "Automated deployment"; first deploy is a
+   manual Run workflow after a verified backup (migrations 0006-0008 are on this branch).
+4. Engineering, unblocked: PR-06 source coverage (verify each provider live before adding).
+5. Blocked on owner: PR-10 off-host backups, PR-08b larger VM / unattended browser policy.
diff --git a/docs/implementation/MASTER_PLAN.md b/docs/implementation/MASTER_PLAN.md
new file mode 100644
index 0000000..0117d7f
--- /dev/null
+++ b/docs/implementation/MASTER_PLAN.md
@@ -0,0 +1,43 @@
+# Modernization master plan
+
+Branch: `modernization/production-readiness` (from `master` @ `1043ab9`). `master` is untouched.
+
+## Baseline (verified 2026-10-27)
+
+- The local working tree held large uncommitted work (single-screen product, CV journey, five
+  profiles, mail outbox, provider fixtures). It is byte-identical to the audit archive
+  `opportunity-platform-audit.tar.gz` except that the archive omits `.env.example` and
+  `.env.prod.example` (archive packaging, not a repository problem). Preserved as commit `06fb29a`.
+- `origin/master` == local `master` == `1043ab9`. The public `master` still has the landing page;
+  only the uncommitted tree removed it.
+- The ranking patch (`opportunity-platform-ranking-20261026.patch`) was never applied. Used as a
+  reference for ranking v2, not copied.
+
+| Area | State at baseline |
+| --- | --- |
+| Rust workspace | builds; 273 tests; 3 failed (stale tests from the WIP), `cargo fmt --check` failed |
+| Frontend | typecheck OK, 3 Vitest tests (local `node_modules` was broken, fixed by `npm ci`) |
+| Browser worker | 19 tests pass (`scripts/verify-browser-worker.sh`) |
+| CV upload in containers | broken: runtime images had no `pdftotext` |
+| Landing page | removed by the WIP; recoverable from `HEAD` (`3d3ef77` introduced it) |
+| Ranking | CV keyword score only; no career stage; profile prefs/blacklist ignored on the dashboard |
+| Shortlist | recency-ordered `LIMIT MATCH_SHORTLIST`; 2,470 open opportunities locally vs. 2,000 cap on e2-micro |
+| Mail | dispatches only `submitted` and `action_required` |
+| Browser worker on real sites | blocked unless `BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised` (deliberate) |
+| CI | `hygiene.yml`: security, Rust+PostgreSQL, frontend, worker. No product acceptance test |
+| CD | scripts exist (`release-image.sh`, `deploy-vm.sh`, `vm-release.sh`, ...), none wired to Actions |
+
+## Roadmap and acceptance
+
+| ID | Work | Acceptance |
+| --- | --- | --- |
+| PR-00 | Baseline, stale tests, pdftotext | full suite green locally |
+| PR-01 | Deterministic acceptance journey in CI | CV → rank → prepare → mock submit → notification → restart, no network |
+| PR-03 | Ranking v2 | profile-specific weights, stage table, eligibility separate, SQL prefilter, tests |
+| PR-04 | Landing restore | original copy/asset/tests; `/`, `/login`, `/workspace`; no private calls on `/` |
+| PR-05 | CV/dossier | CV version pinned per application; facts provenance (partly existing) |
+| PR-07/08 | Submission safety | global daily cap + kill switch independent of employer policies |
+| PR-09 | Notifications | failed / submission_unknown / unsupported delivered |
+| PR-02/10 | CD + backups | build/push/deploy workflow with OIDC, gated by environment; documented bootstrap |
+
+Status lives in `TASK_STATE.md`; resume instructions in `HANDOFF.md`.
diff --git a/docs/implementation/TASK_STATE.md b/docs/implementation/TASK_STATE.md
new file mode 100644
index 0000000..857b45e
--- /dev/null
+++ b/docs/implementation/TASK_STATE.md
@@ -0,0 +1,21 @@
+# Task state
+
+Statuses: NOT_STARTED, IN_PROGRESS, IMPLEMENTED, TESTED, BLOCKED, VERIFIED_COMPLETE.
+TESTED = passes locally. VERIFIED_COMPLETE additionally needs CI or production evidence.
+Nothing on this branch has run in GitHub Actions yet (not pushed).
+
+| ID | Task | Status | Evidence / blocker |
+| --- | --- | --- | --- |
+| PR-00 | Baseline snapshot, stale tests, fmt, pdftotext in images | TESTED | `06fb29a`, `7f00fc2` |
+| PR-01 | Deterministic acceptance journey in CI | TESTED | `ec27d2e`; `TEST_OFFLINE=1 node scripts/test-product.mjs` passes, 0 source runs |
+| PR-03 | Ranking v2 | TESTED | `f0744e8`; 8 unit tests + acceptance stage-order check; `docs/RANKING_POLICY.md` |
+| PR-04 | Landing page restore | TESTED | `2588c32`; 8 Vitest cases; visual check desktop/375 px, light/dark |
+| PR-05 | CV version per application | TESTED | acceptance asserts `cv_document_id` of the submission; existing fact provenance kept |
+| PR-05b | Experience years from date ranges | NOT_STARTED | deliberately deferred, see DECISIONS 5 |
+| PR-07/08 | Owner pause and global daily cap | TESTED | `02371d7`; integration test + UI test |
+| PR-09 | Failure / uncertain / manual-completion emails | TESTED | `6f87dcb`; unit test; acceptance still passes |
+| PR-02 | CD workflow | IMPLEMENTED | `b665d94`; YAML parses, release tests pass; not run: needs bootstrap (BLOCKERS) |
+| PR-10 | Off-host encrypted backups, restore drill | BLOCKED | needs bucket and key decision |
+| PR-06 | Source coverage expansion | NOT_STARTED | each source must be verified against the live provider |
+| PR-08b | Unattended worker on a larger VM | BLOCKED | needs VM size/cost approval and an explicit owner decision on `BROWSER_WORKER_ALLOW_EXTERNAL` |
+| OPS | Read-only VM access check | VERIFIED_COMPLETE | 2026-10-27: `gcloud compute ssh --tunnel-through-iap` ran `hostname && whoami && uptime` as OS Login user `<os-login-user>`; up 4 days, load 0.00. No metadata or profile writes (OS Login on, key already in profile) |
diff --git a/docs/implementation/TEST_RESULTS.md b/docs/implementation/TEST_RESULTS.md
new file mode 100644
index 0000000..e245db9
--- /dev/null
+++ b/docs/implementation/TEST_RESULTS.md
@@ -0,0 +1,29 @@
+# Test results
+
+Only commands actually run are recorded. Local = macOS, Rust 1.92, Node 26, PostgreSQL 16 container.
+
+## 2026-10-27 baseline (`06fb29a`)
+
+- `cargo build --locked --workspace --all-targets`: OK.
+- `cargo test --locked --workspace --no-fail-fast`: 270 passed, 3 failed:
+  - `native_auth::cv_upload_download_and_versions_work...`: 400 vs 201; fixture was an unreadable fake PDF.
+  - `applications::profiles_and_discovery_from_earlier_sessions...`, `discovery::the_four_starting_profiles...`: expected 4 profiles, product seeds 5.
+- `cargo fmt --all -- --check`: failed (WIP formatting). `cargo clippy ... -D warnings`: OK.
+- `apps/web`: `npm run typecheck` OK, `npm test` 3/3 (after `npm ci`; `.bin` links were broken).
+- `scripts/verify-browser-worker.sh`: 19/19.
+
+## 2026-10-27 after repairs (`7f00fc2`)
+
+- `cargo fmt --check`, `cargo clippy -D warnings`: OK. `native_auth` 13/13, core `applications` 23/23, `discovery` 43/43.
+
+## 2026-10-27 at `6f87dcb` / `b665d94`
+
+- `cargo test --locked --workspace --no-fail-fast`: 282 passed, 0 failed. Clippy `-D warnings`, fmt: OK.
+- `apps/web`: typecheck OK, Vitest 13/13 (Home 5, Landing 8).
+- `TEST_OFFLINE=1 node scripts/test-product.mjs` (scratch PostgreSQL, loopback mock ATS and mail): all
+  three PASS lines, `browserErrors: []`, 0 `source_runs`, 0 enabled sources.
+- `scripts/test_release.py` 44 OK, `scripts/test_ci.py` 14 OK, `scripts/test_privacy.py` OK,
+  `scripts/check_privacy.py` PASS. Both workflows parse; actionlint not run locally (CI runs it).
+- Landing checked in the browser pane at desktop and 375 px, light and dark; no horizontal overflow.
+- Not run: GitHub Actions, the deploy workflow, `release-image.sh` (needs a clean checkout of a
+  reviewed SHA; takes a full Docker build), anything against the VM.
diff --git a/migrations/0008_brakes_and_notifications.sql b/migrations/0008_brakes_and_notifications.sql
new file mode 100644
index 0000000..0fe7b9a
--- /dev/null
+++ b/migrations/0008_brakes_and_notifications.sql
@@ -0,0 +1,7 @@
+-- Owner-wide brakes above every employer policy: a pause that stops all submissions, and a daily cap on automatic ones.
+alter table product_settings
+    add column submissions_paused boolean not null default false,
+    add column max_automatic_per_day integer not null default 10 check (max_automatic_per_day between 0 and 100);
+-- Manual completion now reaches the owner as a notification, like failures and uncertain outcomes.
+alter table notifications drop constraint notifications_kind_check;
+alter table notifications add constraint notifications_kind_check check (kind in ('action_required', 'submitted', 'failed', 'submission_uncertain', 'manual_required'));
diff --git a/scripts/test-product.mjs b/scripts/test-product.mjs
index bded1ae..8aa6f67 100644
--- a/scripts/test-product.mjs
+++ b/scripts/test-product.mjs
@@ -8,10 +8,13 @@ import { createRequire } from 'node:module';
 const require=createRequire(resolve('apps/browser-worker/package.json'));
 const {chromium}=require('playwright');
 const name='opportunity_journey_'+Date.now();
-const dbURL=`postgres://opportunity:opportunity_local_dev@127.0.0.1:5432/${name}`;
+// TEST_OFFLINE=1 disables every seed source: no network, so CI is deterministic. Without it, live discovery is a smoke test.
+const offline=process.env.TEST_OFFLINE==='1';
+const dbUser=process.env.TEST_DB_USER||'opportunity',dbPassword=process.env.TEST_DB_PASSWORD||'opportunity_local_dev',dbPort=process.env.TEST_DB_PORT||'5432';
+const dbURL=`postgres://${dbUser}:${dbPassword}@127.0.0.1:${dbPort}/${name}`;
 const dbContainer=process.env.TEST_DB_CONTAINER||'opportunity-db';
 const out=resolve(process.env.TEST_OUTPUT_DIR||'private/product-test');mkdirSync(out,{recursive:true,mode:0o700});
-const sql=(query,db=name)=>execFileSync('docker',['exec','-i',dbContainer,'psql','-U','opportunity','-d',db,'-At','-v','ON_ERROR_STOP=1'],{input:query,encoding:'utf8'}).trim();
+const sql=(query,db=name)=>execFileSync('docker',['exec','-i',dbContainer,'psql','-U',dbUser,'-d',db,'-At','-v','ON_ERROR_STOP=1'],{input:query,encoding:'utf8'}).trim();
 const children=[];let browser;let server;let mail;let submitted=0;let accepted=0;const messages=new Map();
 const sleep=ms=>new Promise(r=>setTimeout(r,ms));
 async function until(fn,timeout=120000){const end=Date.now()+timeout;while(Date.now()<end){const v=await fn();if(v)return v;await sleep(1000);}throw Error('Timed out waiting for product state');}
@@ -22,6 +25,20 @@ try{
  sql(`create database "${name}"`,'postgres');
  await run(['migrate']);
  await run(['user','create','--email','owner@example.test','--name','Test Owner'],'Synthetic-CV-journey-2026!\n');
+ if(offline){await run(['source','import','config/sources.seed.toml']);sql('update sources set enabled=false');}
+ // A synthetic ranking corpus: equal technical evidence at three career stages, plus an unrelated internship.
+ sql(`insert into organizations(id,normalized_name,display_name) values('00000000-0000-4000-8000-000000000011','synthetic ranking employer','Synthetic ranking employer');
+ insert into sources(id,key,display_name,kind,organization_id,endpoint,enabled) values('00000000-0000-4000-8000-000000000012','test-ranking','Synthetic ranking employer','lever','00000000-0000-4000-8000-000000000011','{"kind":"lever","site_id":"ranking"}',false);
+ insert into opportunities(id,organization_id,dedupe_key,canonical_title,kind,advertised_state,search_text) values
+  ('00000000-0000-4000-8000-000000000021','00000000-0000-4000-8000-000000000011','rank-intern','Memory systems intern','internship','advertised','Memory systems intern DRAM CXL Linux kernel C++'),
+  ('00000000-0000-4000-8000-000000000022','00000000-0000-4000-8000-000000000011','rank-junior','Junior memory engineer','employment','advertised','Junior memory engineer DRAM CXL Linux kernel C++'),
+  ('00000000-0000-4000-8000-000000000023','00000000-0000-4000-8000-000000000011','rank-senior','Senior memory engineer','employment','advertised','Senior memory engineer DRAM CXL Linux kernel C++'),
+  ('00000000-0000-4000-8000-000000000024','00000000-0000-4000-8000-000000000011','rank-marketing','Marketing intern','internship','advertised','Marketing intern social media brand');
+ insert into postings(source_id,native_id,opportunity_id,url,title,description_text,content_hash) values
+  ('00000000-0000-4000-8000-000000000012','r1','00000000-0000-4000-8000-000000000021','http://127.0.0.1:8908/apply/single','Memory systems intern','DRAM CXL Linux kernel C++ memory performance','r1'),
+  ('00000000-0000-4000-8000-000000000012','r2','00000000-0000-4000-8000-000000000022','http://127.0.0.1:8908/apply/single','Junior memory engineer','DRAM CXL Linux kernel C++ memory performance','r2'),
+  ('00000000-0000-4000-8000-000000000012','r3','00000000-0000-4000-8000-000000000023','http://127.0.0.1:8908/apply/single','Senior memory engineer','DRAM CXL Linux kernel C++ memory performance','r3'),
+  ('00000000-0000-4000-8000-000000000012','r4','00000000-0000-4000-8000-000000000024','http://127.0.0.1:8908/apply/single','Marketing intern','Social media and brand strategy','r4');`);
  const mock=await import(resolve('apps/browser-worker/dist/mock-server.js'));
  server=await mock.startMockAts(8908);
  mail=createServer(async(req,res)=>{
@@ -38,14 +55,19 @@ try{
  let app=launch(['serve','--port','8897']);
  await until(async()=>{try{return(await fetch(appOrigin+'/ready')).ok;}catch{return false;}});
  browser=await chromium.launch();const page=await browser.newPage({viewport:{width:1150,height:950}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
- await page.goto(appOrigin);await page.getByLabel('Email',{exact:true}).fill('owner@example.test');await page.getByLabel('Password',{exact:true}).fill('Synthetic-CV-journey-2026!');await page.getByRole('button',{name:'Sign in',exact:true}).click();await page.getByLabel('Searching for').waitFor();
+ await page.goto(appOrigin);await page.getByText('bro casually replacing Linkedin').waitFor();await page.getByRole('link',{name:'Log in as Alp (alpblkba)'}).click();await page.getByLabel('Email',{exact:true}).fill('owner@example.test');await page.getByLabel('Password',{exact:true}).fill('Synthetic-CV-journey-2026!');await page.getByRole('button',{name:'Sign in',exact:true}).click();await page.getByLabel('Searching for').waitFor();
  const api=async(path,body)=>page.evaluate(async([path,body])=>{const r=await fetch('/api'+path,body===undefined?{}:{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(body)});if(!r.ok)throw Error(await r.text());return r.status===204?null:r.json();},[path,body]);
  assert.equal((await api('/home')).cv,null);
  const cv='Ada Example\nada@example.test\nPhone: +49 123 4567890\nEducation\nBSc Computer Science, completed 2022\nExperience\nSystems Engineer, 2022 - 2025\n3 years of experience\nLinux kernel, C++, memory performance, firmware and device driver development\nProjects\nCXL memory tiering, DRAM controller, RISC-V FPGA and SystemVerilog\nSkills\nC, C++, Python, Linux, DRAM, CXL, kernel, firmware, RTL, FPGA, RISC-V, SystemVerilog\n';
  await page.getByLabel('Upload CV',{exact:true}).setInputFiles({name:'synthetic-test-cv.txt',mimeType:'text/plain',buffer:Buffer.from(cv)});
  await page.getByText('CV updated. Your matches now use this version.').waitFor();
  let home=await api('/home');assert.equal(home.profiles.length,5);const memory=home.profiles.find(p=>p.name.startsWith('Memory'));await page.getByLabel('Searching for').selectOption(memory.id);
- await until(async()=>{home=await api('/home');return home.discovered>300&&home.jobs.length>5&&home.selected_profile===memory.id;},240000);
+ await until(async()=>{home=await api('/home');return (offline||home.discovered>300&&home.jobs.length>5)&&home.selected_profile===memory.id&&home.jobs.some(j=>j.source_key==='test-ranking');},240000);
+ assert.equal(new URL(page.url()).pathname,'/workspace');
+ const rank=title=>home.jobs.findIndex(j=>j.canonical_title===title);
+ assert.ok(rank('Memory systems intern')>=0&&rank('Memory systems intern')<rank('Junior memory engineer')&&rank('Junior memory engineer')<rank('Senior memory engineer'),JSON.stringify(home.jobs.map(j=>[j.canonical_title,j.score])));
+ assert.equal(rank('Marketing intern'),-1);
+ assert.equal(home.ranking_policy,'career-priority-v2');
  const cvId=home.cv.id;
  await page.reload();await page.getByText('synthetic-test-cv.txt').waitFor();assert.equal((await api('/home')).cv.id,cvId);
  app.kill('SIGTERM');await new Promise(r=>app.on('exit',r));app=launch(['serve','--port','8897']);await until(async()=>{try{return(await fetch(appOrigin+'/ready')).ok;}catch{return false;}});
@@ -53,7 +75,7 @@ try{
  // A replacement changes the candidate record and rankings, without asking for manual facts.
  await page.getByLabel('Upload CV',{exact:true}).setInputFiles({name:'synthetic-test-cv-v2.txt',mimeType:'text/plain',buffer:Buffer.from(cv.replace('3 years','4 years'))});
  await page.getByText('CV updated. Your matches now use this version.').waitFor();home=await api('/home');assert.notEqual(home.cv.id,cvId);
- console.log('PASS login, persistent CV, profile persistence, restart, CV replacement, real discovery and ranking');
+ console.log(`PASS landing, login, persistent CV, profile persistence, restart, CV replacement, ${offline?'synthetic':'real'} discovery and career-stage ranking`);
  await page.screenshot({path:resolve(out,'ranked-desktop.png'),fullPage:false});
  await page.setViewportSize({width:390,height:844});await page.screenshot({path:resolve(out,'ranked-mobile.png'),fullPage:false});
  assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);await page.setViewportSize({width:1150,height:950});
@@ -72,7 +94,7 @@ try{
  await page.getByLabel('Notice period').fill('One month');const noticeForm=page.getByLabel('Notice period').locator('..').locator('..');await noticeForm.getByRole('button',{name:'Save answer'}).click();
  await until(async()=>{home=await api('/home');return home.applications.some(a=>a.opportunity_id===job&&a.status==='submitted');},120000);
  assert.equal(mock.receipts.size,1);submitted=mock.receipts.size;
- const a=home.applications.find(a=>a.opportunity_id===job);await api('/home/apply/'+job,{});await sleep(18000);assert.equal(mock.receipts.size,1);
+ const a=home.applications.find(a=>a.opportunity_id===job);assert.equal(sql(`select cv_document_id from applications where id='${a.id}'`),home.cv.id,'the submission records the CV version it used');await api('/home/apply/'+job,{});await sleep(18000);assert.equal(mock.receipts.size,1);
  await until(async()=>{home=await api('/home');return home.mail.some(m=>m.application_id===a.id&&m.state==='delivered');},120000);
  const submittedEmails=[...messages.values()].filter(m=>m.body.subject.startsWith('Confirmed submitted'));assert.equal(submittedEmails.length,1);assert.match(submittedEmails[0].body.text,/Synthetic test employer/);assert.match(submittedEmails[0].body.text,/http:\/\/127.0.0.1:8908\/apply\/open/);
  console.log('PASS question answers resume, CV attached, confirmed submission, duplicate prevention, one remotely confirmed synthetic email');
~~~~~~

## Appendix C: full diff of the uncommitted working tree

`git diff HEAD` (tracked files), then each untracked file as a new-file diff.

~~~~~~diff
diff --git a/.gitignore b/.gitignore
index 68a7fcd..0eb8c52 100644
--- a/.gitignore
+++ b/.gitignore
@@ -79,3 +79,4 @@ __pycache__/
 /docs/PRE_COMMIT_REVIEW.md
 /docs/GCP_DEPLOYMENT.md
 /docs/E2_MICRO_AUDIT.md
+/docs/implementation/review/
diff --git a/config/sources.seed.toml b/config/sources.seed.toml
index 6c23ac9..cb5dc01 100644
--- a/config/sources.seed.toml
+++ b/config/sources.seed.toml
@@ -135,3 +135,64 @@ site = "NVIDIAExternalCareerSite"
 enabled = true
 refresh_hours = 12
 tags = ["gpu", "systems", "silicon"]
+
+# Added 2026-10-27. Each identifier answered a read-only request with HTTP 200 and a non-empty result
+# (Greenhouse/Ashby job arrays; Workday CXS search total). Intel, Micron and Cadence exceed the
+# adapter's 500-posting sweep ceiling, so their sweeps report `partial` and read the newest first.
+# Not added because the endpoint did not verify: AMD, Qualcomm, Arm, Ampere, Synopsys (Workday 422),
+# Groq, SiFive, Rivos, d-Matrix, Ayar Labs, Enfabrica, Celestial AI (no Greenhouse/Ashby board).
+# Anthropic (Greenhouse) and OpenAI (Ashby) boards verify, but their full listings exceed the HTTP
+# client's 8 MiB response cap sized for the e2-micro, so every sweep would fail; they need a paged
+# or title-only fetch first.
+[[sources]]
+key = "extropic-ashby"
+kind = "ashby"
+organization = "Extropic"
+board_slug = "extropic"
+enabled = true
+refresh_hours = 6
+tags = ["silicon", "ai-accelerator"]
+
+[[sources]]
+key = "intel-workday"
+kind = "workday"
+organization = "Intel"
+tenant = "intel"
+instance = "wd1"
+site = "External"
+enabled = true
+refresh_hours = 12
+tags = ["cpu", "silicon", "systems"]
+
+[[sources]]
+key = "micron-workday"
+kind = "workday"
+organization = "Micron"
+tenant = "micron"
+instance = "wd1"
+site = "External"
+enabled = true
+refresh_hours = 12
+tags = ["memory", "silicon"]
+
+[[sources]]
+key = "marvell-workday"
+kind = "workday"
+organization = "Marvell"
+tenant = "marvell"
+instance = "wd1"
+site = "MarvellCareers"
+enabled = true
+refresh_hours = 12
+tags = ["silicon", "systems"]
+
+[[sources]]
+key = "cadence-workday"
+kind = "workday"
+organization = "Cadence Design Systems"
+tenant = "cadence"
+instance = "wd1"
+site = "External_Careers"
+enabled = true
+refresh_hours = 12
+tags = ["silicon", "verification"]
diff --git a/crates/core/src/mail.rs b/crates/core/src/mail.rs
index 2c06cdd..c8cd175 100644
--- a/crates/core/src/mail.rs
+++ b/crates/core/src/mail.rs
@@ -26,7 +26,7 @@ pub async fn dispatch(db: &Db) -> anyhow::Result<()> {
     sqlx::query("update notifications set delivery_state='failed',delivery_error='Delivery could not be confirmed; automatic retry stopped to prevent duplicate email.' where delivery_receipt is null and first_attempt_at<now()-interval '23 hours' and delivered_at is null")
         .execute(db).await?;
     let mut tx = db.begin().await?;
-    let rows:Vec<(Uuid,String,Value,Option<String>)>=sqlx::query_as("select n.id,n.kind,jsonb_build_object('employer',org.display_name,'position',o.canonical_title,'url',(select p.url from postings p where p.opportunity_id=o.id order by p.url limit 1),'time',n.created_at,'detail',n.payload),n.delivery_receipt from notifications n join applications a on a.id=n.application_id join opportunities o on o.id=a.opportunity_id join organizations org on org.id=o.organization_id where n.kind in ('submitted','action_required','submission_uncertain','failed','manual_required') and n.delivered_at is null and n.delivery_state not in ('bounced','failed') and (n.last_attempt_at is null or n.last_attempt_at<now()-interval '1 minute') order by n.created_at for update of n skip locked limit 5").fetch_all(&mut *tx).await?;
+    let rows:Vec<(Uuid,String,Value,Option<String>)>=sqlx::query_as("select n.id,n.kind,jsonb_build_object('employer',coalesce(org.display_name,n.payload->>'source'),'position',coalesce(o.canonical_title,'career site'),'url',coalesce((select p.url from postings p where p.opportunity_id=o.id order by p.url limit 1),''),'time',n.created_at,'detail',n.payload),n.delivery_receipt from notifications n left join applications a on a.id=n.application_id left join opportunities o on o.id=a.opportunity_id left join organizations org on org.id=o.organization_id where n.kind in ('submitted','action_required','submission_uncertain','failed','manual_required','source_failing') and n.delivered_at is null and n.delivery_state not in ('bounced','failed') and (n.last_attempt_at is null or n.last_attempt_at<now()-interval '1 minute') order by n.created_at for update of n skip locked limit 5").fetch_all(&mut *tx).await?;
     // Persist the attempt before network I/O: a process crash must not reset the retry window.
     for (id, _, _, _) in &rows {
         sqlx::query("update notifications set first_attempt_at=coalesce(first_attempt_at,now()),last_attempt_at=now() where id=$1")
@@ -74,6 +74,7 @@ fn wording(kind: &str) -> (&'static str, &'static str) {
         "submission_uncertain" => ("Check with the employer", "The form may have been sent, but no confirmation was seen. It will not be retried; check with the employer before applying again."),
         "failed" => ("Application failed", "Nothing was confirmed as sent. Open Opportunities to retry it."),
         "manual_required" => ("Complete on the employer website", "Automation cannot finish this form. Open Opportunities for the employer link."),
+        "source_failing" => ("Source not refreshing", "Its listings are kept, not closed, while it fails. Check whether the employer moved its career site."),
         _ => ("Your answer is needed", "Open Opportunities to answer the employer's questions."),
     }
 }
@@ -88,10 +89,11 @@ mod tests {
             "submission_uncertain",
             "failed",
             "manual_required",
+            "source_failing",
         ]
         .iter()
         .map(|k| super::wording(k).0)
         .collect();
-        assert_eq!(subjects.len(), 5);
+        assert_eq!(subjects.len(), 6);
     }
 }
diff --git a/crates/core/src/product.rs b/crates/core/src/product.rs
index 3443897..58f5c60 100644
--- a/crates/core/src/product.rs
+++ b/crates/core/src/product.rs
@@ -769,12 +769,39 @@ pub async fn tick(db: &Db) -> Result<(), AppError> {
     Ok(())
 }
 
+/// Failures this many in a row, after the scheduler's own backoff, mean the source needs the owner.
+const SOURCE_ALERT_FAILURES: i32 = 3;
+
+/// One notification per outage for each enabled source that keeps failing.
+pub async fn alert_failing_sources(db: &Db) -> Result<(), AppError> {
+    let failing: Vec<(Uuid, String, i32, Option<DateTime<Utc>>)> = sqlx::query_as(
+        "select id, display_name, consecutive_failures, last_success_at from sources where enabled and consecutive_failures >= $1",
+    )
+    .bind(SOURCE_ALERT_FAILURES)
+    .fetch_all(db)
+    .await?;
+    for (id, name, failures, last_success) in failing {
+        // Keyed by the last success, so a source that recovers and fails again alerts again.
+        let key = format!(
+            "source-failing:{id}:{}",
+            last_success.map_or(0, |t| t.timestamp())
+        );
+        let payload =
+            json!({"source": name, "failures": failures, "last_success_at": last_success});
+        applications::notify(db, None, "source_failing", &key, payload).await?;
+    }
+    Ok(())
+}
+
 pub fn spawn(db: Db) {
     tokio::spawn(async move {
         loop {
             if let Err(error) = tick(&db).await {
                 tracing::warn!(%error,"application queue");
             }
+            if let Err(error) = alert_failing_sources(&db).await {
+                tracing::warn!(%error, "source alerts");
+            }
             if let Err(error) = crate::mail::dispatch(&db).await {
                 tracing::warn!(%error,"email delivery");
             }
diff --git a/crates/core/tests/applications.rs b/crates/core/tests/applications.rs
index b74a81c..22f40fc 100644
--- a/crates/core/tests/applications.rs
+++ b/crates/core/tests/applications.rs
@@ -1645,3 +1645,54 @@ async fn the_owner_pause_and_daily_cap_refuse_submissions_above_every_policy() {
         .is_none());
     db.cleanup().await;
 }
+
+#[tokio::test]
+async fn a_source_that_keeps_failing_alerts_the_owner_once_per_outage() {
+    let db = require_db!();
+    approved_identity_facts(&db.pool).await;
+    let source: Uuid = sqlx::query_scalar(
+        "insert into sources (key, display_name, kind, endpoint, enabled, consecutive_failures)
+         values ('failing-board', 'Failing Board', 'greenhouse', '{\"kind\":\"greenhouse\",\"board_token\":\"failing\"}', true, 2) returning id",
+    )
+    .fetch_one(&db.pool)
+    .await
+    .unwrap();
+    let alerts = || async {
+        sqlx::query_scalar::<_, i64>(
+            "select count(*) from notifications where kind='source_failing' and application_id is null",
+        )
+        .fetch_one(&db.pool)
+        .await
+        .unwrap()
+    };
+
+    // Below the threshold the scheduler's own retries are still working.
+    opportunity_core::product::alert_failing_sources(&db.pool)
+        .await
+        .unwrap();
+    assert_eq!(alerts().await, 0);
+
+    sqlx::query("update sources set consecutive_failures=3 where id=$1")
+        .bind(source)
+        .execute(&db.pool)
+        .await
+        .unwrap();
+    for _ in 0..2 {
+        opportunity_core::product::alert_failing_sources(&db.pool)
+            .await
+            .unwrap();
+    }
+    assert_eq!(alerts().await, 1, "one alert per outage, not per check");
+
+    // Recovered, then failing again: a new outage.
+    sqlx::query("update sources set last_success_at=now(), consecutive_failures=4 where id=$1")
+        .bind(source)
+        .execute(&db.pool)
+        .await
+        .unwrap();
+    opportunity_core::product::alert_failing_sources(&db.pool)
+        .await
+        .unwrap();
+    assert_eq!(alerts().await, 2);
+    db.cleanup().await;
+}
diff --git a/docs/DEPLOYMENT.md b/docs/DEPLOYMENT.md
index d58eb95..325dc14 100644
--- a/docs/DEPLOYMENT.md
+++ b/docs/DEPLOYMENT.md
@@ -211,6 +211,34 @@ Cloudflare Access assertion validation is tested against assertions signed with
 
 There is no rate limiting at the origin. Cloudflare Access is the only thing in front, which is appropriate for one user but means a leaked assertion is not additionally throttled.
 
-Off-host backup copying is a manual step the owner has to automate; nothing in this repository does it.
+Off-host backups are automated by `.github/workflows/backup.yml` once enabled (below); until then they are a manual step.
+
+## Automated off-host backups
+
+The `backup` workflow runs daily (03:17 UTC) and on demand. `deploy-vm.sh --fetch-backup` streams
+`pg_dump` and the private tree over IAP and encrypts both to the owner's OpenPGP public key on the
+runner before anything is written, so plaintext never reaches a disk off the VM and the VM needs no
+storage credentials. The ciphertext and its `.sha256` go to a private bucket; `gcloud storage cp`
+verifies the upload's checksums. Failed runs are emailed by GitHub. It reuses the deploy identity
+from "Automated deployment".
+
+```bash
+gcloud storage buckets create gs://REPLACE_WITH_BUCKET --project=project-484f9dc3-1a36-4b55-9e8 \
+  --location=us-central1 --uniform-bucket-level-access --public-access-prevention
+# Keep 90 days of daily copies; objects are never overwritten (--no-clobber).
+printf '{"rule":[{"action":{"type":"Delete"},"condition":{"age":90}}]}' > lifecycle.json
+gcloud storage buckets update gs://REPLACE_WITH_BUCKET --lifecycle-file=lifecycle.json
+gcloud storage buckets add-iam-policy-binding gs://REPLACE_WITH_BUCKET --role=roles/storage.objectCreator \
+  --member="serviceAccount:opportunity-deploy@project-484f9dc3-1a36-4b55-9e8.iam.gserviceaccount.com"
+gh variable set BACKUP_BUCKET --body REPLACE_WITH_BUCKET
+gh variable set BACKUP_ENCRYPT_TO --body "<your key's fingerprint>"
+gpg --armor --export "<your key's fingerprint>" | gh variable set BACKUP_GPG_PUBLIC_KEY
+gh variable set BACKUP_ENABLED --body true
+```
+
+`objectCreator` cannot read or delete, so a leaked workflow identity can add copies but not destroy
+them. Restore rehearsal on your own machine (where the private key is): download one
+`opportunity-*.dump.gpg` with its `.sha256` and run `scripts/restore.sh <file>.dump.gpg rehearsal_db`,
+which verifies the checksum and decrypts before restoring into the throwaway database.
 
 Linux amd64 execution was verified under emulation; native amd64 and 1 GiB capacity remain unverified, as described under Architectures.
diff --git a/docs/implementation/BLOCKERS.md b/docs/implementation/BLOCKERS.md
index 74df14d..4072ad4 100644
--- a/docs/implementation/BLOCKERS.md
+++ b/docs/implementation/BLOCKERS.md
@@ -4,6 +4,6 @@
 | --- | --- | --- |
 | GCP Workload Identity Federation pool/provider and deploy service account | automated CD | project id, provider resource name, service account email (repository variables) |
 | GitHub `production` environment with branch rule `master` | gated deploys | repository settings |
-| Off-host backup bucket and encryption key | PR-10 | bucket name, key handling choice |
+| Off-host backup bucket and OpenPGP public key | PR-10 (code ready) | bucket, `BACKUP_*` variables per docs/DEPLOYMENT.md "Automated off-host backups" |
 | Larger VM (2-4 GiB) for Chromium | unattended applications | approval of the monthly cost |
 | Real submission policy | live applications | the owner enabling an employer authorization in the app |
diff --git a/docs/implementation/HANDOFF.md b/docs/implementation/HANDOFF.md
index e1a950b..e497e1a 100644
--- a/docs/implementation/HANDOFF.md
+++ b/docs/implementation/HANDOFF.md
@@ -30,5 +30,7 @@ Do not repeat: the baseline snapshot commit; `npm ci` only if `node_modules` bre
    passphrase is now in the macOS keychain). Any production change still needs the owner's go-ahead.
 3. Owner: GCP/GitHub bootstrap in `docs/DEPLOYMENT.md` "Automated deployment"; first deploy is a
    manual Run workflow after a verified backup (migrations 0006-0008 are on this branch).
-4. Engineering, unblocked: PR-06 source coverage (verify each provider live before adding).
-5. Blocked on owner: PR-10 off-host backups, PR-08b larger VM / unattended browser policy.
+4. Uncommitted in the working tree (owner asked for no commits): new sources, migration 0009 and
+   source alerts, `--fetch-backup` and `backup.yml`. Review with `git status` / `git diff`.
+5. Blocked on owner: backup bucket + public key (docs/DEPLOYMENT.md), PR-08b larger VM / unattended
+   browser policy. Engineering next: paged/title-only fetch for boards over 8 MiB; Workday paging quirk.
diff --git a/docs/implementation/TASK_STATE.md b/docs/implementation/TASK_STATE.md
index 857b45e..6ef3ea3 100644
--- a/docs/implementation/TASK_STATE.md
+++ b/docs/implementation/TASK_STATE.md
@@ -15,7 +15,8 @@ Nothing on this branch has run in GitHub Actions yet (not pushed).
 | PR-07/08 | Owner pause and global daily cap | TESTED | `02371d7`; integration test + UI test |
 | PR-09 | Failure / uncertain / manual-completion emails | TESTED | `6f87dcb`; unit test; acceptance still passes |
 | PR-02 | CD workflow | IMPLEMENTED | `b665d94`; YAML parses, release tests pass; not run: needs bootstrap (BLOCKERS) |
-| PR-10 | Off-host encrypted backups, restore drill | BLOCKED | needs bucket and key decision |
-| PR-06 | Source coverage expansion | NOT_STARTED | each source must be verified against the live provider |
+| PR-10 | Off-host encrypted backups | IMPLEMENTED (uncommitted) | `deploy-vm.sh --fetch-backup` + `.github/workflows/backup.yml`; pipeline round-tripped locally (54 restorable tables); needs bucket, public key and `BACKUP_ENABLED` |
+| PR-06 | Source coverage expansion | TESTED (uncommitted) | Extropic, Intel, Micron, Marvell, Cadence added after a real sweep each; Anthropic/OpenAI exceed the 8 MiB body cap, not added |
+| PR-09b | Alert on sources that keep failing | TESTED (uncommitted) | migration 0009, `product::alert_failing_sources`, integration test (once per outage) |
 | PR-08b | Unattended worker on a larger VM | BLOCKED | needs VM size/cost approval and an explicit owner decision on `BROWSER_WORKER_ALLOW_EXTERNAL` |
 | OPS | Read-only VM access check | VERIFIED_COMPLETE | 2026-10-27: `gcloud compute ssh --tunnel-through-iap` ran `hostname && whoami && uptime` as OS Login user `<os-login-user>`; up 4 days, load 0.00. No metadata or profile writes (OS Login on, key already in profile) |
diff --git a/docs/implementation/TEST_RESULTS.md b/docs/implementation/TEST_RESULTS.md
index e245db9..c401ca6 100644
--- a/docs/implementation/TEST_RESULTS.md
+++ b/docs/implementation/TEST_RESULTS.md
@@ -27,3 +27,13 @@ Only commands actually run are recorded. Local = macOS, Rust 1.92, Node 26, Post
 - Landing checked in the browser pane at desktop and 375 px, light and dark; no horizontal overflow.
 - Not run: GitHub Actions, the deploy workflow, `release-image.sh` (needs a clean checkout of a
   reviewed SHA; takes a full Docker build), anything against the VM.
+
+## 2026-10-27 uncommitted work (sources, source alerts, off-host backups)
+
+- Rust 283/283, fmt and clippy clean; web 13/13; worker 19/19; offline acceptance: three PASS lines, `browserErrors: []`.
+- Real sweeps into a scratch DB: Extropic `ok_complete` (10); Intel, Micron, Cadence `partial` at the
+  500-posting ceiling; Marvell 340 stored but `partial` (existing Workday paging quirk); Anthropic and
+  OpenAI `http_error` (body over the 8 MiB cap), so not added.
+- `deploy-vm.sh --dry-run --fetch-backup` prints the expected remote commands; the inner `pg_dump`
+  piped into `gpg --encrypt` and back decrypted to an archive with 54 restorable tables; a failing
+  dump exits non-zero. Not run against the VM (it would move personal data; owner's key needed).
diff --git a/scripts/deploy-vm.sh b/scripts/deploy-vm.sh
index 2456615..4e84508 100755
--- a/scripts/deploy-vm.sh
+++ b/scripts/deploy-vm.sh
@@ -7,6 +7,7 @@
 #   ./scripts/deploy-vm.sh --rollback opportunity-app:<previous tag>
 #   ./scripts/deploy-vm.sh --status
 #   ./scripts/deploy-vm.sh --reconcile                   # inspect and resolve a pending journal
+#   ./scripts/deploy-vm.sh --fetch-backup                # encrypted dump + private tree into BACKUP_OUT_DIR
 #
 # Transport is Google's IAP tunnel by default, which needs no public address on the VM and no open
 # firewall: the connection is authorized by IAM and tunnelled over Google's front end. Set
@@ -39,6 +40,7 @@ while [[ $# -gt 0 ]]; do
     --preflight) MODE="preflight"; shift ;;
     --rollback) MODE="rollback"; ROLLBACK_TAG="${2:?--rollback needs the image tag to go back to}"; shift 2 ;;
     --status) MODE="status"; shift ;;
+    --fetch-backup) MODE="fetch-backup"; shift ;;
     --reconcile) MODE="reconcile"; shift; if [[ $# -gt 0 && "$1" != -* ]]; then RECONCILE_TOKEN="$1"; shift; fi ;;
     -*) echo "unknown option: $1" >&2; exit 2 ;;
     *) ARCHIVE="$1"; shift ;;
@@ -273,8 +275,36 @@ on_signal() {
 trap on_exit EXIT
 trap on_signal INT TERM HUP
 
+# Read-only on the VM. Plaintext exists only in the pipe: it is encrypted to the owner's public key
+# (BACKUP_ENCRYPT_TO, imported beforehand) before it reaches a disk here.
+fetch_backup() {
+  local out="${BACKUP_OUT_DIR:?set BACKUP_OUT_DIR}" recipient="${BACKUP_ENCRYPT_TO:?set BACKUP_ENCRYPT_TO to the owner public key}"
+  local stamp file
+  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
+  # Single quotes keep $POSTGRES_USER and $POSTGRES_DB for the database container's own environment.
+  local dump='exec -T db sh -c '"'"'pg_dump -U "$POSTGRES_USER" -d "$POSTGRES_DB" --format=custom --no-owner'"'"
+  if [[ "$DRY_RUN" == yes ]]; then
+    remote "cd ${REMOTE_DIR} && ${DOCKER_PRELUDE} $(compose "$dump")"
+    remote "cd ${REMOTE_DIR} && ${DOCKER_PRELUDE} $(compose exec -T app tar -czf - -C /app private)"
+    return 0
+  fi
+  gpg --batch --list-keys "$recipient" >/dev/null || { echo "no public key for ${recipient}; import it first" >&2; return 1; }
+  mkdir -p "$out"
+  remote "cd ${REMOTE_DIR} && ${DOCKER_PRELUDE} $(compose "$dump")" \
+    | gpg --batch --yes --trust-model always --encrypt --recipient "$recipient" --output "${out}/opportunity-${stamp}.dump.gpg"
+  remote "cd ${REMOTE_DIR} && ${DOCKER_PRELUDE} $(compose exec -T app tar -czf - -C /app private)" \
+    | gpg --batch --yes --trust-model always --encrypt --recipient "$recipient" --output "${out}/private-${stamp}.tar.gz.gpg"
+  for file in "${out}/opportunity-${stamp}.dump.gpg" "${out}/private-${stamp}.tar.gz.gpg"; do
+    # An encrypted empty stream is a few hundred bytes; anything that small is a failed dump.
+    [[ "$(wc -c < "$file")" -gt 1024 ]] || { echo "${file} is implausibly small; the backup failed" >&2; return 1; }
+    (cd "$out" && sha256sum "$(basename "$file")" > "$(basename "$file").sha256")
+  done
+  ls -la "$out"
+}
+
 case "$MODE" in
   preflight) preflight; exit $? ;;
+  fetch-backup) fetch_backup; exit $? ;;
   status) vm_action status; exit $? ;;
   reconcile)
     TOKEN="$RECONCILE_TOKEN"
diff --git a/.github/workflows/backup.yml b/.github/workflows/backup.yml
new file mode 100644
index 0000000..a401469
--- /dev/null
+++ b/.github/workflows/backup.yml
@@ -0,0 +1,55 @@
+name: backup
+
+# Daily off-host backup. deploy-vm.sh --fetch-backup streams pg_dump and the private tree (CVs,
+# browser evidence) over IAP and encrypts them here to the owner's public key before they touch a
+# disk; gcloud storage cp then copies the ciphertext to a private bucket and verifies checksums.
+# The private key never leaves the owner. Inert until BACKUP_ENABLED is true (docs/DEPLOYMENT.md).
+on:
+  schedule:
+    - cron: '17 3 * * *'
+  workflow_dispatch:
+
+permissions:
+  contents: read
+  id-token: write
+
+concurrency:
+  group: backup
+  cancel-in-progress: false
+
+defaults:
+  run:
+    shell: bash
+
+jobs:
+  backup:
+    name: Encrypted off-host backup
+    if: vars.BACKUP_ENABLED == 'true'
+    runs-on: ubuntu-24.04
+    timeout-minutes: 30
+    environment: production
+    env:
+      GCP_PROJECT: ${{ vars.GCP_PROJECT }}
+      GCP_INSTANCE: ${{ vars.GCP_INSTANCE }}
+      GCP_ZONE: ${{ vars.GCP_ZONE }}
+      BACKUP_ENCRYPT_TO: ${{ vars.BACKUP_ENCRYPT_TO }}
+      BACKUP_OUT_DIR: ${{ runner.temp }}/backup
+    steps:
+      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
+        with:
+          persist-credentials: false
+      - uses: google-github-actions/auth@7c6bc770dae815cd3e89ee6cdf493a5fab2cc093 # v3
+        with:
+          workload_identity_provider: ${{ vars.GCP_WORKLOAD_IDENTITY_PROVIDER }}
+          service_account: ${{ vars.GCP_DEPLOY_SERVICE_ACCOUNT }}
+      - uses: google-github-actions/setup-gcloud@aa5489c8933f4cc7a4f7d45035b3b1440c9c10db # v3.0.1
+      - name: Import the owner's public key (public half only)
+        env:
+          PUBLIC_KEY: ${{ vars.BACKUP_GPG_PUBLIC_KEY }}
+        run: printf '%s\n' "$PUBLIC_KEY" | gpg --batch --import
+      - name: Stream an encrypted dump and private tree from the VM
+        run: ./scripts/deploy-vm.sh --fetch-backup
+      - name: Copy the ciphertext off-host
+        env:
+          BUCKET: ${{ vars.BACKUP_BUCKET }}
+        run: gcloud storage cp --no-clobber "$BACKUP_OUT_DIR"/* "gs://${BUCKET}/opportunity/"
diff --git a/migrations/0009_source_alerts.sql b/migrations/0009_source_alerts.sql
new file mode 100644
index 0000000..6f04944
--- /dev/null
+++ b/migrations/0009_source_alerts.sql
@@ -0,0 +1,3 @@
+-- A source that keeps failing after the scheduler's own retries reaches the owner, once per outage.
+alter table notifications drop constraint notifications_kind_check;
+alter table notifications add constraint notifications_kind_check check (kind in ('action_required', 'submitted', 'failed', 'submission_uncertain', 'manual_required', 'source_failing'));
~~~~~~

## Appendix D: `docs/DEPLOYMENT.md` (current working tree)

~~~~~~markdown
# Deploying to a small Linux host

This is a single-user private service holding a person's CV, their answers to application questions and the authority to submit applications. The deployment is shaped around that: nothing is published, the origin has no public address, and the application refuses to run in a configuration where personal data would be reachable without an identity check.

Use the owner's separately approved production rollout procedure for the existing Compute Engine release and the owner's private release assessment for current evidence and blockers. The general setup examples below do not authorize production work or an Access policy change. The preflight accessed no production host or Cloudflare account.

## What runs

Three containers. The application is one process that serves the API, the React interface and the background refresh scheduler, because they share a database and splitting them would add operating surface for nothing. PostgreSQL holds everything. A Cloudflare tunnel connects outward so the host needs no inbound firewall rule and the origin has no address to attack. The browser worker is not a long-running service: it is spawned per task and exits.

## Why the origin cannot be reached unprotected

Two independent controls, because one is a configuration and the other is code.

The compose file publishes no ports at all. The only path in is the tunnel, which Cloudflare terminates behind Access.

The application refuses to start on a non-loopback address unless an identity layer is configured. `guard_bind_address` returns an error naming the exact environment variables to set, and the process exits. Binding `0.0.0.0` with `ACCESS_MODE=loopback` is impossible rather than discouraged.

Private API routes require the configured identity: Cloudflare assertions in `cloudflare`, assertions and native sessions in `both`, or native sessions in `native`. Native and both modes serve public interface markup without personal data. An empty Cloudflare allowlist is refused when that identity layer is enabled. `/health` returns liveness and package version; `/ready` additionally checks a database query. Neither identifies the reviewed Git SHA.

## The machine this is going to

The owner's separately approved production rollout procedure covers build identity, IAP transfer, encrypted off-host backup, isolated restoration, authentication stages and recovery. The rest of this file describes the general deployment shape; it is not the procedure for upgrading the existing VM.

## A 1 GiB host

`docker-compose.e2micro.yml` sets the intended 256/192/64 MiB app/database/tunnel limits. The private release assessment distinguishes these limits from measurements; native amd64 validation on a 1 GiB host remains required. The combined profile is:

```bash
docker compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod up -d
```

It sets memory limits, tunes PostgreSQL for a small machine, uses the stock `postgres:16-alpine` image, and runs a prebuilt application image rather than building one — a machine that size cannot compile Rust and should not try. Build elsewhere and bring the image over:

```bash
REVIEWED_SHA="<reviewed-full-commit-sha>" ./scripts/release-image.sh
# Transfer only after the owner approves provenance, off-host restore and recovery-point evidence.
```

The browser is not in that profile. Source sweeps and browser runs are serialized against each other, which removes the failure that used to kill Chromium mid-form, but at the profile's 256 MiB application limit Chromium still has nowhere to run: measured, four attempts submitted nothing and the kernel killed it three times. Assisted application is therefore off there, the process says so at startup, and each application is handed back as a manual one.

To turn it on, build `--target browser` and give the application 448 MiB. On the 1 GiB machine that was enough for six assisted applications in a row with refreshes running underneath, all submitted with verified receipts and no kill — with 92 MB of host memory to spare, which is thin enough that it belongs on a host with 2 GB.

## Choosing a host

The deployment is provider-neutral on purpose. It needs a Linux machine that can run Docker and nothing else: no managed database, no orchestration service, no provider-specific networking. Moving between providers is a `git clone`, an `.env.prod`, and a database restore.

### Sizing

The application idles at well under 200 MB and the memory limits in the compose file cap it at 1 GB, with another 1 GB for PostgreSQL and 256 MB for the tunnel. Two vCPU and 4 GB is comfortable. 2 GB works if nothing else runs on the host, but leave swap configured. Disk: the image is about 180 MB, PostgreSQL holds roughly 25 MB at the current corpus of 1854 opportunities and grows slowly, and raw source snapshots under `private/cache` are the only thing that grows steadily. 20 GB is ample; 40 GB leaves room for backups.

CPU matters only during a refresh sweep and during a browser run. Neither is sustained.

### AWS EC2

A `t4g.small` is arm64 and matches the architecture this image has actually been built and run on. A `t3.small` is amd64. New-customer Free Tier covers a `t2.micro` or `t3.micro` for twelve months, which is 1 GB of memory: that is below what this comfortably needs, so expect to run PostgreSQL tight or to resize.

Use Amazon Linux 2023 or Ubuntu 24.04. Attach a 20 GB or larger gp3 volume. The security group needs **no inbound rule at all**, because the tunnel connects outbound; leaving inbound closed is the point rather than an inconvenience.

```bash
sudo dnf install -y docker git && sudo systemctl enable --now docker   # Amazon Linux 2023
sudo usermod -aG docker "$USER"
sudo curl -fsSL https://github.com/docker/compose/releases/latest/download/docker-compose-linux-$(uname -m)   -o /usr/libexec/docker/cli-plugins/docker-compose && sudo chmod +x /usr/libexec/docker/cli-plugins/docker-compose
```

**The Free Plan is not a hosting plan.** AWS's twelve-month Free Tier and any promotional credits both expire, and when they do the instance keeps running and starts billing. Before relying on it: set a zero-spend budget alert in Billing, note the expiry date, and know that getting your data out is `./scripts/backup.sh` plus copying `backups/` off the host. The database is ordinary PostgreSQL and the dumps are ordinary `pg_dump` custom-format files, so nothing about the data is tied to AWS. Plan the move before the credits run out rather than after.

### Hetzner Cloud

A `CX22` is amd64; a `CAX11` is arm64 and cheaper. Both exceed what this needs. Use Ubuntu 24.04. Create the server with no firewall rules opened for inbound traffic.

```bash
sudo apt-get update && sudo apt-get install -y docker.io docker-compose-plugin git
sudo usermod -aG docker "$USER"
```

Hetzner bills a flat monthly rate with no credit expiry, which makes it the simpler long-term home. Nothing in the deployment differs between the two providers beyond the package manager.

### Architectures

The preflight built and ran the Linux amd64 runtime image under emulation on Apple Silicon, including database and authentication checks. Native amd64 execution and representative total-host memory on a 1 GiB machine remain untested. Explicitly select the runtime target: the Dockerfile also contains a browser stage that does not fit the e2-micro profile.

```bash
docker buildx build --platform linux/amd64 --target runtime -t opportunity-app:amd64 --load .
docker run --rm opportunity-app:amd64 --help
```

## Cloudflare prerequisites the owner provides

A Cloudflare account with the domain on it, a Zero Trust tunnel, and an Access application protecting `opportunities.alpblkba.dev` with the owner's identity and a second factor. DNS for the hostname is created by Cloudflare when the tunnel route is added; no manual record is needed.

## Steps

Everything here needs credentials or DNS, so the owner runs it.

```bash
# On the host, once.
sudo apt-get update && sudo apt-get install -y docker.io docker-compose-plugin git
sudo usermod -aG docker "$USER"   # log out and back in

git clone <your repository> opportunity-platform
cd opportunity-platform
cp .env.prod.example .env.prod
```

Fill in `.env.prod`. Generate the database password with `openssl rand -hex 24`. `CF_ACCESS_TEAM_DOMAIN` is the team hostname such as `example.cloudflareaccess.com`. `CF_ACCESS_AUD` is the Application Audience tag on the Access application's overview page. `ACCESS_ALLOWED_EMAILS` is the owner's address. `CLOUDFLARE_TUNNEL_TOKEN` comes from the tunnel's install screen. `.env.prod` is git-ignored and must stay off the repository.

In the Cloudflare tunnel, add a public hostname routing `opportunities.alpblkba.dev` to `http://app:8787`, and attach the Access application to that hostname.

```bash
docker compose -f docker-compose.prod.yml --env-file .env.prod up -d --build
docker compose -f docker-compose.prod.yml --env-file .env.prod ps
docker compose -f docker-compose.prod.yml --env-file .env.prod logs app | head -20
```

The log should name the access mode and the number of allowed identities. If it says loopback, the service will have refused to bind and the container will be restarting.

## Moving the existing data

The local database holds roughly 1854 opportunities plus source, profile and application state. `scripts/migrate-to-cloud.sh` exports it, loads it into the deployed database and compares row counts per table. It only reads the local database.

```bash
./scripts/migrate-to-cloud.sh
```

Rehearsed locally on 2026-10-22: 1854 opportunities, 1905 postings, 14 sources, 4 profiles, 5 applications and 7 facts all transferred with zero `pg_restore` errors and every table matching.

## Backups

```bash
./scripts/backup.sh                                    # database dump plus the private tree, with a checksum
./scripts/restore.sh backups/<file>.dump rehearsal_db  # into a throwaway name
```

`restore.sh` verifies the checksum first and refuses to overwrite an existing database unless `ALLOW_OVERWRITE=yes` is passed, because the usual reason to run it is a rehearsal and the usual disaster is pointing it at the live database. Rehearsed locally: restored into a disposable database, row counts matched, and both real databases were untouched.

`scripts/backup.sh` writes files under `BACKUP_DIR` (default `backups/`) on the same machine, which protects against a mistaken `drop` and not against losing the host. The dumps contain the CV, the answers and the application history, so they must leave the host: a cron entry running `rsync` or `rclone` to storage elsewhere, encrypted at rest. Verify one occasionally by restoring it into a throwaway database. A backup nobody has restored is a hypothesis.

## The scheduler

State lives in PostgreSQL: each source carries `next_run_at`, a failure count and a backoff. A restart resumes rather than re-scanning, which was verified across a container restart.

One advisory lock serializes everything heavy: the background sweep, a hand-triggered refresh, and any run of the browser. A sweep that finds it held skips and comes back on its next tick; a browser run waits up to a minute and then reports that the machine is busy. The lock is held on a connection detached from the pool, so waiting costs no pool slot and a process that is killed releases it by dying.

The `app` service is pinned to one replica, and a refresh additionally takes a PostgreSQL advisory lock before it runs. The lock is what actually prevents two sweeps of one source overlapping, whether they come from the background loop and a manual refresh in one process or from two processes entirely. A second sweep finds the lock held and skips rather than queues. The replica pin remains because one scheduler is all this workload needs, not because the lock is missing.

## Semantic ranking

Off unless an embedding provider is reachable from the server. The owner's MacBook is not a provider for the server and must not become a dependency: `EMBEDDINGS_PROVIDER=disabled` is the default, and the interface says "lexical ranking only" wherever a score appears. To turn it on later, run an embedding model on the host itself and point `EMBEDDINGS_ENDPOINT` at it.

## What is still off

Live employer submission. The browser worker reaches loopback only; opening that gate needs `BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised` and the owner present. Automatic submission additionally needs a standing policy the owner creates and enables, which is created switched off.

## Updating, rolling back and recovering

Follow the owner's separately approved production rollout procedure. Build the exact reviewed release elsewhere, retain the actual previous image, create encrypted database and private-file backups, copy them off-host and prove an isolated restore before migration. Do not build on the e2-micro or use `migrate-to-cloud.sh` to upgrade existing data.

Migrations run before the server binds, in transactions recorded by SQLx. Migration 0005 preserves legacy rows, but the older runtime refuses its unknown migration history. Automatic image recovery cannot reverse that history. Recovery to the pre-0005 binary requires a new database restored from the pre-release backup and the matching retained deployment files/checker; it must preserve the failed database for investigation. Do not edit migration history or overwrite the live database to bypass this gate.

## Automated deployment

`.github/workflows/deploy.yml` runs after `continuous integration` succeeds on a push to `master`:
it builds the reviewed archive with `release-image.sh`, then runs `deploy-vm.sh --preflight` and
`deploy-vm.sh` over IAP, then `--status`. A release that changes `migrations/` since the last
deployed revision is refused automatically; deploy it on purpose with **Actions → deploy → Run
workflow** after checking the backup. Failed runs are emailed by GitHub. The workflow does nothing
until `DEPLOY_ENABLED` is `true`.

One-time bootstrap by the owner (no keys are created; GitHub receives short-lived tokens):

```bash
PROJECT=project-484f9dc3-1a36-4b55-9e8
NUMBER="$(gcloud projects describe "$PROJECT" --format='value(projectNumber)')"
gcloud iam workload-identity-pools create github --project="$PROJECT" --location=global
gcloud iam workload-identity-pools providers create-oidc opportunity-platform --project="$PROJECT" \
  --location=global --workload-identity-pool=github --issuer-uri=https://token.actions.githubusercontent.com \
  --attribute-mapping=google.subject=assertion.sub,attribute.repository=assertion.repository,attribute.environment=assertion.environment \
  --attribute-condition="assertion.repository=='alpblkba/opportunity-platform' && assertion.environment=='production'"
gcloud iam service-accounts create opportunity-deploy --project="$PROJECT"
SA="opportunity-deploy@${PROJECT}.iam.gserviceaccount.com"
gcloud iam service-accounts add-iam-policy-binding "$SA" --project="$PROJECT" --role=roles/iam.workloadIdentityUser \
  --member="principalSet://iam.googleapis.com/projects/${NUMBER}/locations/global/workloadIdentityPools/github/attribute.repository/alpblkba/opportunity-platform"
for role in roles/compute.viewer roles/iap.tunnelResourceAccessor roles/compute.osAdminLogin; do
  gcloud projects add-iam-policy-binding "$PROJECT" --member="serviceAccount:${SA}" --role="$role"
done
```

If the VM runs as a service account, also grant `roles/iam.serviceAccountUser` on that account to
`$SA`. OS Login must be enabled on the instance (`enable-oslogin=TRUE`); check with
`gcloud compute instances describe opportunity-platform --zone us-central1-a --format='value(metadata.items)'`
before changing it, because without OS Login `gcloud compute ssh` writes keys into instance metadata.

Then in GitHub: create the `production` environment limited to the `master` branch, and set:

```bash
gh variable set GCP_PROJECT --body project-484f9dc3-1a36-4b55-9e8
gh variable set GCP_INSTANCE --body opportunity-platform
gh variable set GCP_ZONE --body us-central1-a
gh variable set GCP_DEPLOY_SERVICE_ACCOUNT --body "opportunity-deploy@project-484f9dc3-1a36-4b55-9e8.iam.gserviceaccount.com"
gh variable set GCP_WORKLOAD_IDENTITY_PROVIDER --body "projects/${NUMBER}/locations/global/workloadIdentityPools/github/providers/opportunity-platform"
gh variable set DEPLOY_ENABLED --body true
```

The first workflow deployment must be a manual **Run workflow**, which establishes the deployed
revision the schema check compares against.

## Known limitations

Cloudflare Access assertion validation is tested against assertions signed with a test key held in the repository: the positive path accepts a correctly signed, correctly addressed, unexpired token for an allowlisted identity, and the negative paths refuse a wrong audience, a wrong issuer, an expired token, an unknown key id, a tampered payload, an identity outside the allowlist, a token with no identity, `alg: none` and garbage. The deployed instance has since been signed into with a real Access one-time code and returned authenticated API responses, so the team domain, the key endpoint and the audience tag are confirmed as well as the verification logic.

There is no rate limiting at the origin. Cloudflare Access is the only thing in front, which is appropriate for one user but means a leaked assertion is not additionally throttled.

Off-host backups are automated by `.github/workflows/backup.yml` once enabled (below); until then they are a manual step.

## Automated off-host backups

The `backup` workflow runs daily (03:17 UTC) and on demand. `deploy-vm.sh --fetch-backup` streams
`pg_dump` and the private tree over IAP and encrypts both to the owner's OpenPGP public key on the
runner before anything is written, so plaintext never reaches a disk off the VM and the VM needs no
storage credentials. The ciphertext and its `.sha256` go to a private bucket; `gcloud storage cp`
verifies the upload's checksums. Failed runs are emailed by GitHub. It reuses the deploy identity
from "Automated deployment".

```bash
gcloud storage buckets create gs://REPLACE_WITH_BUCKET --project=project-484f9dc3-1a36-4b55-9e8 \
  --location=us-central1 --uniform-bucket-level-access --public-access-prevention
# Keep 90 days of daily copies; objects are never overwritten (--no-clobber).
printf '{"rule":[{"action":{"type":"Delete"},"condition":{"age":90}}]}' > lifecycle.json
gcloud storage buckets update gs://REPLACE_WITH_BUCKET --lifecycle-file=lifecycle.json
gcloud storage buckets add-iam-policy-binding gs://REPLACE_WITH_BUCKET --role=roles/storage.objectCreator \
  --member="serviceAccount:opportunity-deploy@project-484f9dc3-1a36-4b55-9e8.iam.gserviceaccount.com"
gh variable set BACKUP_BUCKET --body REPLACE_WITH_BUCKET
gh variable set BACKUP_ENCRYPT_TO --body "<your key's fingerprint>"
gpg --armor --export "<your key's fingerprint>" | gh variable set BACKUP_GPG_PUBLIC_KEY
gh variable set BACKUP_ENABLED --body true
```

`objectCreator` cannot read or delete, so a leaked workflow identity can add copies but not destroy
them. Restore rehearsal on your own machine (where the private key is): download one
`opportunity-*.dump.gpg` with its `.sha256` and run `scripts/restore.sh <file>.dump.gpg rehearsal_db`,
which verifies the checksum and decrypts before restoring into the throwaway database.

Linux amd64 execution was verified under emulation; native amd64 and 1 GiB capacity remain unverified, as described under Architectures.
~~~~~~

## Appendix E: tracker docs and ranking policy (current working tree)

### `docs/implementation/MASTER_PLAN.md`

~~~~~~markdown
# Modernization master plan

Branch: `modernization/production-readiness` (from `master` @ `1043ab9`). `master` is untouched.

## Baseline (verified 2026-10-27)

- The local working tree held large uncommitted work (single-screen product, CV journey, five
  profiles, mail outbox, provider fixtures). It is byte-identical to the audit archive
  `opportunity-platform-audit.tar.gz` except that the archive omits `.env.example` and
  `.env.prod.example` (archive packaging, not a repository problem). Preserved as commit `06fb29a`.
- `origin/master` == local `master` == `1043ab9`. The public `master` still has the landing page;
  only the uncommitted tree removed it.
- The ranking patch (`opportunity-platform-ranking-20261026.patch`) was never applied. Used as a
  reference for ranking v2, not copied.

| Area | State at baseline |
| --- | --- |
| Rust workspace | builds; 273 tests; 3 failed (stale tests from the WIP), `cargo fmt --check` failed |
| Frontend | typecheck OK, 3 Vitest tests (local `node_modules` was broken, fixed by `npm ci`) |
| Browser worker | 19 tests pass (`scripts/verify-browser-worker.sh`) |
| CV upload in containers | broken: runtime images had no `pdftotext` |
| Landing page | removed by the WIP; recoverable from `HEAD` (`3d3ef77` introduced it) |
| Ranking | CV keyword score only; no career stage; profile prefs/blacklist ignored on the dashboard |
| Shortlist | recency-ordered `LIMIT MATCH_SHORTLIST`; 2,470 open opportunities locally vs. 2,000 cap on e2-micro |
| Mail | dispatches only `submitted` and `action_required` |
| Browser worker on real sites | blocked unless `BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised` (deliberate) |
| CI | `hygiene.yml`: security, Rust+PostgreSQL, frontend, worker. No product acceptance test |
| CD | scripts exist (`release-image.sh`, `deploy-vm.sh`, `vm-release.sh`, ...), none wired to Actions |

## Roadmap and acceptance

| ID | Work | Acceptance |
| --- | --- | --- |
| PR-00 | Baseline, stale tests, pdftotext | full suite green locally |
| PR-01 | Deterministic acceptance journey in CI | CV → rank → prepare → mock submit → notification → restart, no network |
| PR-03 | Ranking v2 | profile-specific weights, stage table, eligibility separate, SQL prefilter, tests |
| PR-04 | Landing restore | original copy/asset/tests; `/`, `/login`, `/workspace`; no private calls on `/` |
| PR-05 | CV/dossier | CV version pinned per application; facts provenance (partly existing) |
| PR-07/08 | Submission safety | global daily cap + kill switch independent of employer policies |
| PR-09 | Notifications | failed / submission_unknown / unsupported delivered |
| PR-02/10 | CD + backups | build/push/deploy workflow with OIDC, gated by environment; documented bootstrap |

Status lives in `TASK_STATE.md`; resume instructions in `HANDOFF.md`.
~~~~~~

### `docs/implementation/TASK_STATE.md`

~~~~~~markdown
# Task state

Statuses: NOT_STARTED, IN_PROGRESS, IMPLEMENTED, TESTED, BLOCKED, VERIFIED_COMPLETE.
TESTED = passes locally. VERIFIED_COMPLETE additionally needs CI or production evidence.
Nothing on this branch has run in GitHub Actions yet (not pushed).

| ID | Task | Status | Evidence / blocker |
| --- | --- | --- | --- |
| PR-00 | Baseline snapshot, stale tests, fmt, pdftotext in images | TESTED | `06fb29a`, `7f00fc2` |
| PR-01 | Deterministic acceptance journey in CI | TESTED | `ec27d2e`; `TEST_OFFLINE=1 node scripts/test-product.mjs` passes, 0 source runs |
| PR-03 | Ranking v2 | TESTED | `f0744e8`; 8 unit tests + acceptance stage-order check; `docs/RANKING_POLICY.md` |
| PR-04 | Landing page restore | TESTED | `2588c32`; 8 Vitest cases; visual check desktop/375 px, light/dark |
| PR-05 | CV version per application | TESTED | acceptance asserts `cv_document_id` of the submission; existing fact provenance kept |
| PR-05b | Experience years from date ranges | NOT_STARTED | deliberately deferred, see DECISIONS 5 |
| PR-07/08 | Owner pause and global daily cap | TESTED | `02371d7`; integration test + UI test |
| PR-09 | Failure / uncertain / manual-completion emails | TESTED | `6f87dcb`; unit test; acceptance still passes |
| PR-02 | CD workflow | IMPLEMENTED | `b665d94`; YAML parses, release tests pass; not run: needs bootstrap (BLOCKERS) |
| PR-10 | Off-host encrypted backups | IMPLEMENTED (uncommitted) | `deploy-vm.sh --fetch-backup` + `.github/workflows/backup.yml`; pipeline round-tripped locally (54 restorable tables); needs bucket, public key and `BACKUP_ENABLED` |
| PR-06 | Source coverage expansion | TESTED (uncommitted) | Extropic, Intel, Micron, Marvell, Cadence added after a real sweep each; Anthropic/OpenAI exceed the 8 MiB body cap, not added |
| PR-09b | Alert on sources that keep failing | TESTED (uncommitted) | migration 0009, `product::alert_failing_sources`, integration test (once per outage) |
| PR-08b | Unattended worker on a larger VM | BLOCKED | needs VM size/cost approval and an explicit owner decision on `BROWSER_WORKER_ALLOW_EXTERNAL` |
| OPS | Read-only VM access check | VERIFIED_COMPLETE | 2026-10-27: `gcloud compute ssh --tunnel-through-iap` ran `hostname && whoami && uptime` as OS Login user `<os-login-user>`; up 4 days, load 0.00. No metadata or profile writes (OS Login on, key already in profile) |
~~~~~~

### `docs/implementation/DECISIONS.md`

~~~~~~markdown
# Decisions

1. **Snapshot the uncommitted tree as its own commit** (`06fb29a`) on a feature branch, so every
   later change is a reviewable diff and nothing the owner wrote can be lost.
2. **Keep the stack** (Rust/Axum, React, PostgreSQL, Playwright, Compose, Actions, GCE VM,
   Cloudflare). Nothing found justifies a change.
3. **Keep the external-navigation guard** in the browser worker. Unattended real submissions stay a
   deliberate owner switch (`BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised`), not a default.
4. **Shortlist truncation is fixed by a SQL prefilter on the profile's direction terms**, not by
   raising the limit or caching rankings: `fit()` already discards rows without those terms.
5. **Experience years stay explicit-claim only.** Summing employment date ranges would let
   telecom years clear an RTL "3+ years" gap; without domain attribution it is either wrong or
   changes nothing. Revisit only with owner-confirmed per-domain experience facts.
6. **Research profile uses the four technical areas as its direction terms.** Its old terms were
   stage words, which gave any internship 40 technical points.
7. **Ranking weights live in code (`product.rs`), versioned by `RANKING_POLICY`.** No runtime config
   until someone needs to change them without a release.
8. **Pause and daily cap sit in `workflow::submit`**, the one path every real submission takes; the
   cap is checked inside the browser slot, which already serializes submissions.
9. **No registry**: CD reuses the archive-over-IAP release path the scripts already verify.
   Schema-changing releases are manual because an older image refuses a newer migration history.
~~~~~~

### `docs/implementation/TEST_RESULTS.md`

~~~~~~markdown
# Test results

Only commands actually run are recorded. Local = macOS, Rust 1.92, Node 26, PostgreSQL 16 container.

## 2026-10-27 baseline (`06fb29a`)

- `cargo build --locked --workspace --all-targets`: OK.
- `cargo test --locked --workspace --no-fail-fast`: 270 passed, 3 failed:
  - `native_auth::cv_upload_download_and_versions_work...`: 400 vs 201; fixture was an unreadable fake PDF.
  - `applications::profiles_and_discovery_from_earlier_sessions...`, `discovery::the_four_starting_profiles...`: expected 4 profiles, product seeds 5.
- `cargo fmt --all -- --check`: failed (WIP formatting). `cargo clippy ... -D warnings`: OK.
- `apps/web`: `npm run typecheck` OK, `npm test` 3/3 (after `npm ci`; `.bin` links were broken).
- `scripts/verify-browser-worker.sh`: 19/19.

## 2026-10-27 after repairs (`7f00fc2`)

- `cargo fmt --check`, `cargo clippy -D warnings`: OK. `native_auth` 13/13, core `applications` 23/23, `discovery` 43/43.

## 2026-10-27 at `6f87dcb` / `b665d94`

- `cargo test --locked --workspace --no-fail-fast`: 282 passed, 0 failed. Clippy `-D warnings`, fmt: OK.
- `apps/web`: typecheck OK, Vitest 13/13 (Home 5, Landing 8).
- `TEST_OFFLINE=1 node scripts/test-product.mjs` (scratch PostgreSQL, loopback mock ATS and mail): all
  three PASS lines, `browserErrors: []`, 0 `source_runs`, 0 enabled sources.
- `scripts/test_release.py` 44 OK, `scripts/test_ci.py` 14 OK, `scripts/test_privacy.py` OK,
  `scripts/check_privacy.py` PASS. Both workflows parse; actionlint not run locally (CI runs it).
- Landing checked in the browser pane at desktop and 375 px, light and dark; no horizontal overflow.
- Not run: GitHub Actions, the deploy workflow, `release-image.sh` (needs a clean checkout of a
  reviewed SHA; takes a full Docker build), anything against the VM.

## 2026-10-27 uncommitted work (sources, source alerts, off-host backups)

- Rust 283/283, fmt and clippy clean; web 13/13; worker 19/19; offline acceptance: three PASS lines, `browserErrors: []`.
- Real sweeps into a scratch DB: Extropic `ok_complete` (10); Intel, Micron, Cadence `partial` at the
  500-posting ceiling; Marvell 340 stored but `partial` (existing Workday paging quirk); Anthropic and
  OpenAI `http_error` (body over the 8 MiB cap), so not added.
- `deploy-vm.sh --dry-run --fetch-backup` prints the expected remote commands; the inner `pg_dump`
  piped into `gpg --encrypt` and back decrypted to an archive with 54 restorable tables; a failing
  dump exits non-zero. Not run against the VM (it would move personal data; owner's key needed).
~~~~~~

### `docs/implementation/BLOCKERS.md`

~~~~~~markdown
# Blockers (external, need the owner)

| Blocker | Needed for | What the owner must provide |
| --- | --- | --- |
| GCP Workload Identity Federation pool/provider and deploy service account | automated CD | project id, provider resource name, service account email (repository variables) |
| GitHub `production` environment with branch rule `master` | gated deploys | repository settings |
| Off-host backup bucket and OpenPGP public key | PR-10 (code ready) | bucket, `BACKUP_*` variables per docs/DEPLOYMENT.md "Automated off-host backups" |
| Larger VM (2-4 GiB) for Chromium | unattended applications | approval of the monthly cost |
| Real submission policy | live applications | the owner enabling an employer authorization in the app |
~~~~~~

### `docs/implementation/DEPLOYMENT_READINESS.md`

~~~~~~markdown
# Deployment readiness

| Question | Answer (2026-10-27) |
| --- | --- |
| Is automatic deployment implemented? | Yes, `.github/workflows/deploy.yml`. |
| Is it connected to GitHub/GCP? | No. Needs the bootstrap in `docs/DEPLOYMENT.md` "Automated deployment" and `DEPLOY_ENABLED`. |
| Has a real deployment been performed from this branch? | No. Nothing was pushed or deployed. |
| Is the live application healthy? | Not checked: no read-only VM access from this machine yet (`gcloud` missing). |
| Schema changes on this branch | `0006`, `0007` (owner WIP), `0008` (brakes, notification kinds). The first deploy must be a manual run after a verified backup; image rollback cannot undo them. |
| Runtime image change | `poppler-utils` added so PDF CVs can be read in containers. |
| Browser automation in production | Still off on e2-micro; external navigation still needs `BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised`. |
| Backups | Scripted on the VM; off-host copy still manual (`BLOCKERS.md`). |
~~~~~~

### `docs/implementation/HANDOFF.md`

~~~~~~markdown
# Handoff

Read this, then `TASK_STATE.md`, then `git status` / `git log --oneline -15`.

- Branch: `modernization/production-readiness` (local only, not pushed). Never commit to `master`;
  never force-push. `master` == `origin/master` == `1043ab9`, untouched.
- Commits: `06fb29a` owner WIP snapshot · `7f00fc2` baseline repairs · `10addad` tracker ·
  `f0744e8` ranking v2 · `2588c32` landing · `ec27d2e` CI acceptance · `02371d7` pause/cap ·
  `6f87dcb` failure emails · `b665d94` deploy workflow · then this tracker update.
- Local test database: a disposable container, never the owner's `opportunity-db` on 5432.

```bash
docker run -d --rm --name opportunity-ci-scratch -e POSTGRES_USER=opportunity_ci \
  -e POSTGRES_PASSWORD=synthetic_ci_only -e POSTGRES_DB=opportunity_ci -p 127.0.0.1:55432:5432 postgres:16-alpine
export DATABASE_URL=postgres://opportunity_ci:synthetic_ci_only@127.0.0.1:55432/opportunity_ci
cargo fmt --all -- --check && cargo clippy --locked --workspace --all-targets -- -D warnings && cargo test --locked --workspace
(cd apps/web && npm ci && npm run typecheck && npm test && npm run build)
bash scripts/verify-browser-worker.sh
cargo build --locked --bin opportunity && TEST_OFFLINE=1 TEST_DB_CONTAINER=opportunity-ci-scratch \
  TEST_DB_USER=opportunity_ci TEST_DB_PASSWORD=synthetic_ci_only TEST_DB_PORT=55432 node scripts/test-product.mjs
```

Do not repeat: the baseline snapshot commit; `npm ci` only if `node_modules` breaks.

## Next actions

1. Owner: push the branch and open a PR so CI (including the new acceptance job) runs on GitHub.
2. Done: read-only IAP access works (`gcloud` 590.0.1 via Homebrew; prepend
   `/opt/homebrew/share/google-cloud-sdk/bin` to PATH; the GCE key must be in ssh-agent, and its
   passphrase is now in the macOS keychain). Any production change still needs the owner's go-ahead.
3. Owner: GCP/GitHub bootstrap in `docs/DEPLOYMENT.md` "Automated deployment"; first deploy is a
   manual Run workflow after a verified backup (migrations 0006-0008 are on this branch).
4. Uncommitted in the working tree (owner asked for no commits): new sources, migration 0009 and
   source alerts, `--fetch-backup` and `backup.yml`. Review with `git status` / `git diff`.
5. Blocked on owner: backup bucket + public key (docs/DEPLOYMENT.md), PR-08b larger VM / unattended
   browser policy. Engineering next: paged/title-only fetch for boards over 8 MiB; Workday paging quirk.
~~~~~~

### `docs/RANKING_POLICY.md`

~~~~~~markdown
# Opportunity ranking (`career-priority-v2`)

The dashboard's priority is a ranking heuristic for this owner, not a hiring probability or a
qualification score. It is computed per request from the selected profile and the current CV
(`crates/core/src/product.rs`).

## Order of operations

1. **Candidate set.** Only open opportunities whose search text contains one of the profile's
   direction terms are loaded, so the `MATCH_SHORTLIST` cap never drops an older relevant posting
   behind newer irrelevant ones.
2. **Technical fit (0-100).** Direction terms in the title (or two in the body), CV skills found in
   the posting, CV projects/employment, minus 10 per gap. Below 20 the posting is dropped; no stage
   bonus can rescue it. The research profile uses the four technical areas' terms; stage words
   ("internship", "thesis") give no technical credit.
3. **Hard exclusions.** The saved profile's job kinds, location preferences and global or
   per-profile blacklist rules drop the posting.
4. **Review gaps.** Experience years, required PhD, work authorization wording, title-only
   listings, and PhD posts without a confirmed vacancy mark the posting `needs_review`. Unknown is
   never treated as eligible.
5. **Priority** = `t * technical + p * profile + s * career_stage`, clamped to 0-100.

| Profile | t | p | s |
| --- | ---: | ---: | ---: |
| firmware, memory, architecture, ai-systems | 0.55 | 0.15 | 0.30 |
| research | 0.45 | 0.20 | 0.35 |

| Career stage (from kind or title only) | Engineering | Research |
| --- | ---: | ---: |
| Master's thesis | 100 | 100 |
| Internship | 100 | 85 |
| Working student / student role | 92 | 80 |
| Research assistant | 92 | 92 |
| PhD, advertised vacancy | 65 | 95 |
| PhD, topic or supervisor page only | 65 | 60 |
| New graduate / early career | 78 | 72 |
| Junior / Engineer I / associate | 74 | 68 |
| Level not stated | 52 | 52 |
| Mid / Engineer II / 5-7 years required | 36 | 36 |
| Senior / lead / 8+ years required | 8 | 8 |
| Staff / principal / director | 2 | 2 |

These values are an initial policy, not calibrated against outcomes.

## Automatic applications

Ranking never authorizes a submission. Auto-queueing requires the unboosted CV match to be strong
with no gaps, the saved-profile assessment to pass, an advertised vacancy, and a separate,
unexpired employer authorization created by the owner.
~~~~~~

## Appendix F: owner WIP snapshot `06fb29a` (file stat)

Full patch: `wip-snapshot.patch` in this directory (`git diff 1043ab9 06fb29a`).

~~~~~~text
 .env.example                                      |  15 +-
 Cargo.lock                                        |   1 +
 README.md                                         |  26 +-
 apps/browser-worker/mock/multi-step.html          |   2 +-
 apps/browser-worker/mock/open-questions.html      |   2 +-
 apps/browser-worker/mock/single-step.html         |   4 +-
 apps/browser-worker/src/providers.test.ts         |  25 +
 apps/browser-worker/src/providers.ts              |  54 +++
 apps/browser-worker/src/worker.test.ts            |  42 +-
 apps/browser-worker/src/worker.ts                 | 155 +++----
 apps/web/public/linkedin-vibe.webp                | Bin 22878 -> 0 bytes
 apps/web/src/App.tsx                              | 346 +++-----------
 apps/web/src/__tests__/App.test.tsx               | 148 ------
 apps/web/src/__tests__/Home.test.tsx              |  37 ++
 apps/web/src/__tests__/Landing.test.tsx           | 178 -------
 apps/web/src/__tests__/Login.test.tsx             | 175 -------
 apps/web/src/__tests__/Matches.test.tsx           | 225 ---------
 apps/web/src/__tests__/ProductJourney.test.tsx    |  56 ---
 apps/web/src/__tests__/Workspace.test.tsx         | 233 ----------
 apps/web/src/__tests__/format.test.ts             | 106 -----
 apps/web/src/api.ts                               | 236 ++--------
 apps/web/src/components/ApplicationTracking.tsx   |  34 --
 apps/web/src/components/CoveragePage.tsx          | 143 ------
 apps/web/src/components/CvPanel.tsx               |  37 --
 apps/web/src/components/DiscoverPage.tsx          | 264 -----------
 apps/web/src/components/LandingPage.tsx           |  67 ---
 apps/web/src/components/LoginPage.tsx             |   4 +-
 apps/web/src/components/MatchesPage.tsx           | 282 ------------
 apps/web/src/components/ProfilesPage.tsx          | 383 ----------------
 apps/web/src/components/SaveApplicationButton.tsx |  16 -
 apps/web/src/components/SourcesPage.tsx           | 273 -----------
 apps/web/src/components/WorkspacePage.tsx         | 536 ----------------------
 apps/web/src/format.ts                            | 110 -----
 apps/web/src/styles.css                           | 488 +-------------------
 apps/web/src/types.ts                             | 381 ---------------
 config/sources.seed.toml                          |  12 +
 crates/adapters/src/workday.rs                    |  35 +-
 crates/api/Cargo.toml                             |   1 +
 crates/api/src/applications.rs                    |  31 +-
 crates/api/src/lib.rs                             |   1 +
 crates/api/src/product.rs                         |  61 +++
 crates/api/src/routes.rs                          |   3 +-
 crates/cli/src/main.rs                            |   2 +
 crates/core/src/applications.rs                   |  42 +-
 crates/core/src/candidate.rs                      | 253 ++++++++++
 crates/core/src/documents.rs                      |  49 +-
 crates/core/src/lib.rs                            |   3 +
 crates/core/src/mail.rs                           |  68 +++
 crates/core/src/matching.rs                       |   9 +-
 crates/core/src/product.rs                        | 531 +++++++++++++++++++++
 crates/core/src/profiles.rs                       |  72 ++-
 crates/core/src/scheduler.rs                      |  14 +-
 crates/core/src/worker.rs                         |  16 +-
 crates/core/src/workflow.rs                       |  56 ++-
 crates/core/tests/applications.rs                 |  25 +
 crates/domain/src/application.rs                  |  35 +-
 crates/domain/src/normalize.rs                    |   2 +-
 migrations/0006_cv_journey.sql                    |  18 +
 migrations/0007_email_attempt_window.sql          |   2 +
 run.sh                                            |   4 +
 scripts/test-cv-pdf.mjs                           |  23 +
 scripts/test-mail.mjs                             |  54 +++
 scripts/test-product.mjs                          |  88 ++++
 63 files changed, 1743 insertions(+), 4851 deletions(-)
~~~~~~
