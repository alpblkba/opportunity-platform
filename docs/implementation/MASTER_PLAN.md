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
