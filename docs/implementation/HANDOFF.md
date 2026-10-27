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

1. Uncommitted in the working tree (owner: "do not commit or push anything"): the first-pass work
   (sources, 0009, `--fetch-backup`) plus the second pass (TASK_STATE S2-01..S2-10). Review with
   `git status` / `git diff`. `docs/implementation/review/` is a git-ignored export from before the
   second pass; it is stale.
2. Pre-commit corrections done (S2-11..S2-13). A commit plan was proposed to the owner; nothing is
   committed until the owner authorizes it. Then push and open a PR so CI runs in GitHub.
3. First rollout: docs/implementation/DEPLOYMENT_READINESS.md "Go / no-go", in order.
4. Owner decisions: TOTP or password-only before the auth cutover (docs/AUTHENTICATION.md
   "Cutover assessment"); WIF bootstrap for automated CD; larger VM for unattended applications.
5. Engineering next, if wanted: TOTP for native login; a paged Workday sweep beyond 500 postings;
   recovering OpenAI (Ashby has no titles-only form); domain-attributed experience facts (PR-05b).
6. Disposable local resources from this session: containers `opportunity-ci-scratch` (55432) and
   `opportunity-review-db` (55433), the `review-native` preview on 8788, and `.claude/review-login.txt`.
   Stop the containers with `docker stop` (they were started with `--rm`); delete the login file.

VM access: `gcloud` 590.0.1 via Homebrew (prepend `/opt/homebrew/share/google-cloud-sdk/bin`), the GCE
key in ssh-agent with its passphrase in the macOS keychain. Read-only use only without the owner's
go-ahead.
