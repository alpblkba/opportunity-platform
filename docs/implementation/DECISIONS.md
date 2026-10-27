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
10. **Production state is read from the VM, never from Actions history.** The running image's revision
    label and SQLx's checksums decide deploys (`release-gate.py`); a false Actions success cannot move it.
11. **Migration rollouts run from the owner's machine only**, with a backup under 24 hours old that
    passed `restore-rehearsal.sh`. Actions deploys code-only releases. No registry, still.
12. **Backups are pulled to the owner's Mac** (owner decision): encrypted to a pinned fingerprint,
    staged and promoted atomically. `backup.yml` stays, off, as the optional future path.
13. **Every standing policy is bound to the opportunity's own open posting** at exactly that URL and
    source, and the automatic queue re-ranks right before submitting; ranking never authorizes.
14. **Status changes go through one guarded UPDATE** (`allowed_from`). Owner tracking edits stay
    separate (`update_tracking`), and the attempts table remains the durable duplicate guard.
15. **A required number of years is always for review**; preferred years add no gap. Domain-attributed
    experience is still not modelled (DECISIONS 5 stands; PR-05b stays deferred).
16. **The release seed is created at most once per key** (`seed_ledger`), so owner deletions survive
    restarts and upgrades; an explicit `opportunity source import` may still recreate.
17. **Large Greenhouse boards fall back to the documented titles-only listing** instead of raising the
    8 MiB cap; Ashby has no such form, so OpenAI stays unsupported.
18. **Eligibility gates every standing-policy attempt in `workflow::submit`**, whichever path it came by.
    A one-time owner token is the explicit override and is not held.
19. **Owner and system status changes are separate, explicit functions.** Closed applications reopen
    only through owner tracking, then explicit queueing; tracking takes an application out of automation.
20. **A submission's outcome is one transaction** (attempt, status, event, notification), and
    `recover_interrupted` reconciles from attempt rows with the same notification keys.
