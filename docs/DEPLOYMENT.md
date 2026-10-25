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

## Known limitations

Cloudflare Access assertion validation is tested against assertions signed with a test key held in the repository: the positive path accepts a correctly signed, correctly addressed, unexpired token for an allowlisted identity, and the negative paths refuse a wrong audience, a wrong issuer, an expired token, an unknown key id, a tampered payload, an identity outside the allowlist, a token with no identity, `alg: none` and garbage. The deployed instance has since been signed into with a real Access one-time code and returned authenticated API responses, so the team domain, the key endpoint and the audience tag are confirmed as well as the verification logic.

There is no rate limiting at the origin. Cloudflare Access is the only thing in front, which is appropriate for one user but means a leaked assertion is not additionally throttled.

Off-host backup copying is a manual step the owner has to automate; nothing in this repository does it.

Linux amd64 execution was verified under emulation; native amd64 and 1 GiB capacity remain unverified, as described under Architectures.
