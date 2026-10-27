# Task state

Statuses: NOT_STARTED, IN_PROGRESS, IMPLEMENTED, TESTED_LOCALLY, VERIFIED_IN_CI,
VERIFIED_IN_PRODUCTION, BLOCKED. Nothing on this branch has run in GitHub Actions (not pushed), so
nothing is VERIFIED_IN_CI; production still runs `1043ab9`, so nothing here is VERIFIED_IN_PRODUCTION.

Committed on the branch (`06fb29a..053693d`) vs. uncommitted working tree is noted per row.

| ID | Task | Status | Evidence / blocker |
| --- | --- | --- | --- |
| PR-00 | Baseline snapshot, stale tests, fmt, pdftotext in images | TESTED_LOCALLY | `06fb29a`, `7f00fc2` |
| PR-01 | Deterministic acceptance journey in CI | TESTED_LOCALLY | `ec27d2e`; offline journey passes, 0 source runs |
| PR-03 | Ranking v2 | TESTED_LOCALLY | `f0744e8`; `docs/RANKING_POLICY.md` |
| PR-04 | Landing page restore | TESTED_LOCALLY | `2588c32`; Vitest + local review instance |
| PR-05 | CV version per application | TESTED_LOCALLY | acceptance asserts the submission's `cv_document_id` |
| PR-07/08 | Owner pause and global daily cap | TESTED_LOCALLY | `02371d7` |
| PR-09 | Failure / uncertain / manual-completion emails | TESTED_LOCALLY | `6f87dcb` |
| PR-02 | CD workflow | TESTED_LOCALLY (uncommitted rework) | gate replaced run-history check; see S2-01 |
| PR-06 | Source coverage expansion | TESTED_LOCALLY (uncommitted) | 6 sources (Anthropic added in the second pass), each swept for real |
| PR-09b | Alert on sources that keep failing | TESTED_LOCALLY (uncommitted) | migration `0009` |
| PR-10 | Encrypted backups pulled to the owner's Mac | TESTED_LOCALLY (uncommitted) | see S2-02; never run against the VM (owner key needed) |
| S2-01 | Release gate on production's real state | TESTED_LOCALLY (uncommitted) | `scripts/release-gate.py`, `vm-release.sh status` machine lines, `deploy-vm.sh --gate/--migration-rollout`, `deploy.yml`; `GateTests` 14, `DeployGateTests` 8 |
| S2-02 | Backup hardening and restore rehearsal | TESTED_LOCALLY (uncommitted) | `.incomplete` staging, fingerprint pinning, manifest, `scripts/restore-rehearsal.sh`; `BackupTests` 4; `scripts/test-backup-roundtrip.py` real round trip PASS |
| S2-03 | Read-only preflight, explicit `--check-transfer` | TESTED_LOCALLY (uncommitted) | `DeployGateTests.test_preflight_is_read_only...` |
| S2-04 | Submission authorization, destination binding, SSRF | TESTED_LOCALLY (uncommitted) | no-profile policies now bound to the opportunity's own open posting; `destination_is_safe`; queue re-checks eligibility; 4 new integration tests + unit test |
| S2-05 | Application state transitions and retry safety | TESTED_LOCALLY (uncommitted) | `allowed_from` in one guarded UPDATE; status gate before any attempt; failed-after-click becomes unknown |
| S2-06 | Ranking recall, stage classification, required vs preferred | TESTED_LOCALLY (uncommitted) | `career-priority-v3`; relevance-ordered shortlist (old ordering dropped the needle at 5k/10k); 27 title variants; `crates/core/tests/ranking.rs` |
| S2-07 | Dashboard counts as aggregates | TESTED_LOCALLY (uncommitted) | `product::application_counts`; 260-application test; Vitest |
| S2-08 | Seed reconciliation (no resurrection), Workday completeness | TESTED_LOCALLY (uncommitted) | `seed_ledger` in 0009; Workday first-total fix (Marvell live `ok_complete` 343/343); Greenhouse titles-only fallback (Anthropic live `ok_complete` 679) |
| S2-09 | Source outage lifecycle with reminders | TESTED_LOCALLY (uncommitted) | reminders at 1/3/7/14/30 days then monthly; never-succeeded no longer silenced |
| S2-10 | Ops status, cloudflared pin, CI review | TESTED_LOCALLY (uncommitted) | `ops::status` (`/api/ops/status`, `opportunity ops`), failing-source notice in the UI, `APP_REVISION`; cloudflared pinned to the running digest, `vm-check.sh` matches by reference; worker verifier fixed for Node 26; CI trust boundaries reviewed, unchanged |
| S2-11 | Standing-policy eligibility enforced centrally | TESTED_LOCALLY (uncommitted) | `product::policy_eligibility_hold` called by `workflow::submit` before any policy attempt; owner tokens stay the override; `crates/api/tests/submission_policy.rs` over real HTTP |
| S2-12 | No status write outside explicit rules | TESTED_LOCALLY (uncommitted) | `queue_for_automation`, `resume_after_answers`, tracking clears `auto_resume`, recovery records events; queue skips unresolved attempts |
| S2-13 | Atomic submission outcomes and crash reconciliation | TESTED_LOCALLY (uncommitted) | `finish_submission` (one transaction), `recover_interrupted`; half-finished and pre-commit crashes tested |
| AUTH | Public landing page via native cutover | BLOCKED | owner decision (TOTP or password-only) and a Cloudflare policy change; assessment in docs/AUTHENTICATION.md |
| PR-05b | Experience years from date ranges | NOT_STARTED | deliberately deferred, DECISIONS 5 |
| PR-08b | Unattended worker on a larger VM | BLOCKED | VM size/cost approval; `BROWSER_WORKER_ALLOW_EXTERNAL` owner decision |
| OPS | Read-only VM access | VERIFIED_IN_PRODUCTION | IAP + OS Login; status and label reads 2026-10-27, no writes |
