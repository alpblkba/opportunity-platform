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

**Owner decision (2026-10-27): backups are pulled to the owner's Mac.** Nothing is written on the VM,
so the e2-micro's disk is never involved, and the VM holds no storage credentials.

```bash
gpg --fingerprint you@example.com            # once: note the 40-hex fingerprint of your own key
BACKUP_OUT_DIR=~/Backups/opportunity BACKUP_ENCRYPT_TO=<FINGERPRINT> ./scripts/deploy-vm.sh --fetch-backup
./scripts/restore-rehearsal.sh ~/Backups/opportunity/opportunity-<stamp>
```

- `--fetch-backup` streams `pg_dump` and the private tree (CVs, browser evidence) over IAP and encrypts
  both on the Mac to the key named by its full fingerprint; a name, email or short id is refused, as is
  a revoked, expired or encrypt-incapable key. Everything lands in a mode-700 `.incomplete-*` directory
  that becomes `opportunity-<stamp>/` only after both streams were checked as a real `pg_dump`/gzip
  stream, hashed, and written with a `manifest.json` (production revision, migration checksums,
  plaintext and ciphertext SHA-256). Any failure removes it; earlier backups are never touched.
- The dump is taken first, then the files. Uploads only ever add files, so every CV the dump references
  is in the archive; a CV uploaded between the two is an extra file, not a missing one.
- `restore-rehearsal.sh` verifies the checksums, decrypts with your key, restores into a throwaway
  `postgres:16-alpine` container with no network, unpacks the private tree into a temporary directory,
  and checks every referenced CV's bytes. Success writes `restore-verified.json`, which a migration
  rollout requires. It never touches a real database.
- **This is not continuous off-host protection:** a backup exists only when the Mac runs it. Run it
  before every deploy (a migration rollout refuses a backup older than 24 hours) and periodically.
- Later, optionally: `.github/workflows/backup.yml` (off until `BACKUP_ENABLED`) runs the same fetch
  daily from GitHub Actions to a private bucket; see "Optional automated off-host backups". It costs a
  bucket and needs the deploy identity; nothing is provisioned until the owner decides.

`scripts/backup.sh` and `scripts/restore.sh` remain for a host-local copy and for restoring a single
dump; `restore.sh` refuses to overwrite an existing database unless `ALLOW_OVERWRITE=yes`.

## The scheduler

State lives in PostgreSQL: each source carries `next_run_at`, a failure count and a backoff. A restart resumes rather than re-scanning, which was verified across a container restart.

One advisory lock serializes everything heavy: the background sweep, a hand-triggered refresh, and any run of the browser. A sweep that finds it held skips and comes back on its next tick; a browser run waits up to a minute and then reports that the machine is busy. The lock is held on a connection detached from the pool, so waiting costs no pool slot and a process that is killed releases it by dying.

The `app` service is pinned to one replica, and a refresh additionally takes a PostgreSQL advisory lock before it runs. The lock is what actually prevents two sweeps of one source overlapping, whether they come from the background loop and a manual refresh in one process or from two processes entirely. A second sweep finds the lock held and skips rather than queues. The replica pin remains because one scheduler is all this workload needs, not because the lock is missing.

## Semantic ranking

Off unless an embedding provider is reachable from the server. The owner's MacBook is not a provider for the server and must not become a dependency: `EMBEDDINGS_PROVIDER=disabled` is the default, and the interface says "lexical ranking only" wherever a score appears. To turn it on later, run an embedding model on the host itself and point `EMBEDDINGS_ENDPOINT` at it.

## What is still off

Live employer submission. The browser worker reaches loopback only; opening that gate needs `BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised` and the owner present. Automatic submission additionally needs a standing policy the owner creates and enables, which is created switched off.

## Updating, rolling back and recovering

Every deploy, from a laptop or from Actions, first asks the VM what it actually runs
(`vm-release.sh status`: the running image's `org.opencontainers.image.revision` label and SQLx's own
`_sqlx_migrations` checksums) and lets `scripts/release-gate.py` decide. It refuses when production
has an unresolved or in-progress release, runs a revision this repository does not know (an
out-of-band build), or runs something the candidate does not descend from; when the candidate is not
on `origin/master`; when an applied migration is missing from the candidate or was edited after it
ran; and when a migration would apply out of order. Re-deploying the running revision does nothing.

A release that **adds** migrations is never automatic, because the older image refuses a newer
migration history and image rollback therefore cannot undo it. Deploy it from your machine:

```bash
BACKUP_OUT_DIR=~/Backups/opportunity BACKUP_ENCRYPT_TO=<FINGERPRINT> ./scripts/deploy-vm.sh --fetch-backup
./scripts/restore-rehearsal.sh ~/Backups/opportunity/opportunity-<stamp>
REVIEWED_SHA="$(git rev-parse HEAD)" ./scripts/release-image.sh          # clean checkout of a master commit
REVIEWED_SHA="$(git rev-parse HEAD)" ./scripts/deploy-vm.sh --migration-rollout ~/Backups/opportunity/opportunity-<stamp> \
  "dist/opportunity-app-$(git rev-parse HEAD)-amd64.tar.gz"
./scripts/deploy-vm.sh --status
```

The gate accepts the backup only if it was taken from the revision and migration history production
runs now, is under 24 hours old, matches its manifest, and passed the rehearsal. If the migrated
release then fails, the previous image will not start against the migrated schema: restore the
verified backup into a new database for the previous image and keep the migrated one for
investigation. Never edit `_sqlx_migrations` to get past this.

`--preflight` is read-only. `--check-transfer` is the one check that writes (an empty file over scp,
removed again), and says so. Do not build on the e2-micro or use `migrate-to-cloud.sh` to upgrade
existing data.

## Automated deployment

`.github/workflows/deploy.yml` runs after `continuous integration` succeeds on a push to `master`
(or by hand on `master`): it re-checks that the exact SHA is on master and green, authenticates with
Workload Identity Federation, runs `deploy-vm.sh --gate` against production's real state, and only
then builds with `release-image.sh`, preflights and deploys over IAP and reports `--status`. A
repeated trigger for the running revision ends at the gate. A release that adds migrations is refused
here, manual runs included: it needs a verified backup only the owner's machine holds (above). Failed
runs are emailed by GitHub. The workflow does nothing until `DEPLOY_ENABLED` is `true`.

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

Activate it only after the first migration-bearing release has been rolled out by hand as above and
CI has passed in GitHub: from then on ordinary code-only merges deploy themselves.

## Known limitations

Cloudflare Access assertion validation is tested against assertions signed with a test key held in the repository: the positive path accepts a correctly signed, correctly addressed, unexpired token for an allowlisted identity, and the negative paths refuse a wrong audience, a wrong issuer, an expired token, an unknown key id, a tampered payload, an identity outside the allowlist, a token with no identity, `alg: none` and garbage. The deployed instance has since been signed into with a real Access one-time code and returned authenticated API responses, so the team domain, the key endpoint and the audience tag are confirmed as well as the verification logic.

In `cloudflare` mode nothing at the origin throttles requests carrying a valid assertion; native login (`both`/`native`) has its own per-address limiter (see docs/AUTHENTICATION.md), and a Cloudflare rate-limiting rule is recommended before any cutover.

Backups exist only when the owner's Mac pulls one (above); the optional workflow below is off.

The tunnel image is pinned to the cloudflared build production ran on 2026-10-27 (2026.10.3, by
digest). A deploy restarts only `app`, so the pin takes effect when the tunnel is next recreated, with
the same image.

## Optional automated off-host backups

Off by the owner's decision; nothing here is provisioned.

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
gh variable set BACKUP_ENCRYPT_TO --body "<your key's full 40-hex fingerprint>"
gpg --armor --export "<your key's fingerprint>" | gh variable set BACKUP_GPG_PUBLIC_KEY
gh variable set BACKUP_ENABLED --body true
```

`objectCreator` cannot read or delete, so a leaked workflow identity can add copies but not destroy
them. Restore rehearsal happens on your own machine, where the private key is: download one
`opportunity-<stamp>/` directory and run `scripts/restore-rehearsal.sh` on it.

Linux amd64 execution was verified under emulation; native amd64 and 1 GiB capacity remain unverified, as described under Architectures.
