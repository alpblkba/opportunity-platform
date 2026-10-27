# Blockers (external, need the owner)

| Blocker | Needed for | What the owner must provide |
| --- | --- | --- |
| Permission to commit and push this work | CI in GitHub, any deploy | review; "commit" / "push" (currently "do not commit or push anything") |
| An OpenPGP key on the Mac (its fingerprint) | backups, the first migration rollout | `gpg --quick-generate-key` or an existing key; nothing leaves the Mac |
| GCP Workload Identity Federation pool/provider, deploy service account, IAM | automated CD | the commands in docs/DEPLOYMENT.md "Automated deployment" |
| GitHub `production` environment (branch rule `master`), `GCP_*` variables, `DEPLOY_ENABLED` | gated automated deploys | repository settings, after the first rollout |
| TOTP vs. password-only decision; Cloudflare policy swap and login rate-limit rule | public landing page | decision, then the Zero Trust dashboard |
| Larger VM (2-4 GiB) and `BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised` | unattended applications | approval of the monthly cost; explicit decision |
| Real submission policy | live applications | enabling an employer authorization in the app |
| `actionlint` locally (optional) | local workflow lint | approval to install it (CLAUDE.md); CI runs it anyway |
| Off-host backup bucket (optional) | continuous off-host backups | a decision and a bucket; `backup.yml` stays off until then |
