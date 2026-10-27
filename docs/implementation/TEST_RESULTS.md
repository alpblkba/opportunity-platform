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
  piped into `gpg --encrypt` and back decrypted; a failing dump exits non-zero. (The "54" reported
  then counted dump entries, not tables; the real round trip below counts 27-28 public tables.)
- Correction: "worker 19/19" above was the test count; on this Mac's Node 26 the verifier itself
  exited 1, because Node 26 ignores the reporter flag placed after the test files. Fixed below.

## 2026-10-27 second pass (uncommitted; everything local, nothing in Actions or production)

Commands run on the final tree (disposable PostgreSQL 16 on 55432; the owner's 5432 and app on 8787
untouched):

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check`; `cargo clippy --locked --workspace --all-targets -- -D warnings` | clean |
| `cargo test --locked --workspace --no-fail-fast` | **297 passed, 0 failed** (adapters 59, api 14+11+13, core 39 unit + applications 31 + discovery 44 + ranking 2, domain 84) |
| `apps/web`: `npm run typecheck`, `npx vitest run`, `npm run build` | OK, **15/15**, built |
| `bash scripts/verify-browser-worker.sh` | exit 0, **19/19** |
| `python3 -I scripts/test_release.py` | **70 OK** (44 earlier + GateTests 14, DeployGateTests 8, BackupTests 4) |
| `python3 -I scripts/test_ci.py` / `test_privacy.py` / `check_privacy.py` | 15 OK / OK / PASS (163 paths) |
| `python3 -I scripts/test-backup-roundtrip.py` | PASS: real pg_dump -> gpg -> rehearsal restore, 28 tables, 9 migrations, CV verified; truncated, missing archive and wrong key refused |
| `TEST_OFFLINE=1 ... node scripts/test-product.mjs` | three PASS lines, `browserErrors: []` |
| `bash -n` on every script; workflow YAML parse (Ruby) | OK |
| Ranking at 500/2,000/5,000/10,000 with a 2,000 cap | old internship first at every size (41/136/271/331 ms on this Mac); the previous ordering dropped it at 5,000 |
| Live read-only sweeps into a throwaway DB | Marvell `ok_complete` 343/343 (was `partial`); Anthropic `ok_complete` 679 title-only |
| Local review instance, native mode, browser | landing unchanged; login -> workspace; logout -> landing; `/workspace` signed out -> form; `/api/ops/status` 401 without session; no overflow at 412 px; no server errors |

Not run: GitHub Actions (not pushed), `actionlint` (not installed here; CI downloads it with a pinned
checksum), `release-image.sh` and the linux/amd64 image (so `pdftotext` in the image is checked in
the Dockerfile, not executed), anything mutating against the VM, native amd64 on 1 GiB.

## 2026-10-27 pre-commit corrections (uncommitted)

After centralizing the policy eligibility gate, closing direct status writes and making outcomes atomic:

| Command | Result |
| --- | --- |
| `cargo fmt --all -- --check`; `cargo clippy --locked --workspace --all-targets -- -D warnings` | clean (exit 0) |
| `cargo test --locked --workspace --no-fail-fast` | **300 passed, 0 failed** (new: api `submission_policy` 1, core `applications` +2) |
| `apps/web`: typecheck, `npx vitest run`, `npm run build` | OK, 15/15, built |
| `bash scripts/verify-browser-worker.sh` | exit 0, 19/19 |
| `test_release.py` / `test_ci.py` / `test_privacy.py` / `check_privacy.py` | 70 OK / 15 OK / 23 OK / PASS (163 paths) |
| `TEST_OFFLINE=1 ... node scripts/test-product.mjs` | three PASS lines, `browserErrors: []` |
| `python3 -I scripts/test-backup-roundtrip.py` | PASS (28 tables, 9 migrations, CV verified; three damaged backups refused) |
| `bash -n` all scripts; workflow YAML parse | OK |
| `actionlint` | **not run** (not installed locally) |
