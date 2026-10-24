# Deploying to a small Linux host

This is a single-user private service holding a person's CV, their answers to application questions and the authority to submit applications. The deployment is shaped around that: nothing is published, the origin has no public address, and the application refuses to run in a configuration where personal data would be reachable without an identity check.

Nothing in this document has been executed against a real host or a real Cloudflare account. Everything below was verified locally against the same image and the same access mode, with the tunnel omitted because starting one would expose the origin. The owner runs the steps that need credentials, billing or DNS.

## What runs

Three containers. The application is one process that serves the API, the React interface and the background refresh scheduler, because they share a database and splitting them would add operating surface for nothing. PostgreSQL holds everything. A Cloudflare tunnel connects outward so the host needs no inbound firewall rule and the origin has no address to attack. The browser worker is not a long-running service: it is spawned per task and exits.

## Why the origin cannot be reached unprotected

Two independent controls, because one is a configuration and the other is code.

The compose file publishes no ports at all. The only path in is the tunnel, which Cloudflare terminates behind Access.

The application refuses to start on a non-loopback address unless an identity layer is configured. `guard_bind_address` returns an error naming the exact environment variables to set, and the process exits. Binding `0.0.0.0` with `ACCESS_MODE=loopback` is impossible rather than discouraged.

Every route under `/api`, and the interface itself, runs through one middleware that validates the Cloudflare Access assertion against the team's public keys, checks the audience and issuer, and compares the identity against an allowlist. An empty allowlist is refused at startup rather than meaning everyone. `/health` stays open so a container orchestrator can probe it; it returns a status and a version and nothing else.

## The machine this is going to

`docs/GCP_DEPLOYMENT.md` is the runbook for the Compute Engine instance that already exists: what to build, how to get it there over IAP, what to check, how to roll back, and exactly what access that needs. The rest of this file is the general shape of the deployment and the providers it was sized against.

## A 1 GiB host

`docs/E2_MICRO_AUDIT.md` measures this deployment on a machine with 1 GiB of memory, such as a Google Compute Engine `e2-micro`, and `docker-compose.e2micro.yml` is the profile it measures:

```bash
docker compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod up -d
```

It sets memory limits, tunes PostgreSQL for a small machine, uses the stock `postgres:16-alpine` image, and runs a prebuilt application image rather than building one — a machine that size cannot compile Rust and should not try. Build elsewhere and bring the image over:

```bash
docker build -t opportunity-app:latest --target runtime .      # on a machine with a toolchain
docker save opportunity-app:latest | ssh <host> docker load    # or push to a registry and pull
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

The image is defined for both architectures and the Rust and Node toolchains it uses support both. **Only `linux/arm64` has actually been built and run**, on Apple Silicon. `linux/amd64` has not been built or started in this session and is therefore unverified. Before trusting an amd64 host, build it explicitly and check the container reaches a healthy state:

```bash
docker buildx build --platform linux/amd64 -t opportunity-app:amd64 --load .
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

Fill in `.env.prod`. Generate the database password with `openssl rand -base64 32`. `CF_ACCESS_TEAM_DOMAIN` is the team hostname such as `example.cloudflareaccess.com`. `CF_ACCESS_AUD` is the Application Audience tag on the Access application's overview page. `ACCESS_ALLOWED_EMAILS` is the owner's address. `CLOUDFLARE_TUNNEL_TOKEN` comes from the tunnel's install screen. `.env.prod` is git-ignored and must stay off the repository.

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

`scripts/backup.sh` writes to a named volume on the same machine, which protects against a mistaken `drop` and not against losing the host. The dumps contain the CV, the answers and the application history, so they must leave the host: a cron entry running `rsync` or `rclone` to storage elsewhere, encrypted at rest. Verify one occasionally by restoring it into a throwaway database. A backup nobody has restored is a hypothesis.

## The scheduler

State lives in PostgreSQL: each source carries `next_run_at`, a failure count and a backoff. A restart resumes rather than re-scanning, which was verified across a container restart.

One advisory lock serializes everything heavy: the background sweep, a hand-triggered refresh, and any run of the browser. A sweep that finds it held skips and comes back on its next tick; a browser run waits up to a minute and then reports that the machine is busy. The lock is held on a connection detached from the pool, so waiting costs no pool slot and a process that is killed releases it by dying.

The `app` service is pinned to one replica, and a refresh additionally takes a PostgreSQL advisory lock before it runs. The lock is what actually prevents two sweeps of one source overlapping, whether they come from the background loop and a manual refresh in one process or from two processes entirely. A second sweep finds the lock held and skips rather than queues. The replica pin remains because one scheduler is all this workload needs, not because the lock is missing.

## Semantic ranking

Off unless an embedding provider is reachable from the server. The owner's MacBook is not a provider for the server and must not become a dependency: `EMBEDDINGS_PROVIDER=disabled` is the default, and the interface says "lexical ranking only" wherever a score appears. To turn it on later, run an embedding model on the host itself and point `EMBEDDINGS_ENDPOINT` at it.

## What is still off

Live employer submission. The browser worker reaches loopback only; opening that gate needs `BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised` and the owner present. Automatic submission additionally needs a standing policy the owner creates and enables, which is created switched off.

## Updating, rolling back and recovering

An update is a pull and a rebuild. Compose replaces the application container and leaves the database and its volume alone.

```bash
./scripts/backup.sh                      # always, before an update
git pull
docker compose -f docker-compose.prod.yml --env-file .env.prod up -d --build app
docker compose -f docker-compose.prod.yml --env-file .env.prod logs -f app
```

Migrations run at startup and are applied in a transaction each, so a failed migration leaves the schema at the last good state and the container restarts rather than serving a half-migrated database.

To roll back, check out the previous commit and rebuild the same way. A rollback across a migration that changed the schema is the one case needing the backup: restore the dump taken before the update into a fresh database and point `DATABASE_URL` at it, because an older binary will not understand a newer schema.

If a deployment fails outright and the container will not stay up, `docker compose ... logs app` names the reason. The common causes are a missing or wrong `.env.prod` value, which the application refuses to start on rather than guessing, and an unreachable database, which it retries. Nothing is lost in either case: the volumes outlive the containers.

Recovering a host from nothing: provision, install Docker, clone, write `.env.prod`, `up -d`, then `./scripts/restore.sh` the most recent dump into the live database with `ALLOW_OVERWRITE=yes`. The private tree is restored from its own archive. Budget twenty minutes, most of it the image build.

## Known limitations

Cloudflare Access assertion validation is tested against assertions signed with a test key held in the repository: the positive path accepts a correctly signed, correctly addressed, unexpired token for an allowlisted identity, and the negative paths refuse a wrong audience, a wrong issuer, an expired token, an unknown key id, a tampered payload, an identity outside the allowlist, a token with no identity, `alg: none` and garbage. The deployed instance has since been signed into with a real Access one-time code and returned authenticated API responses, so the team domain, the key endpoint and the audience tag are confirmed as well as the verification logic.

There is no rate limiting at the origin. Cloudflare Access is the only thing in front, which is appropriate for one user but means a leaked assertion is not additionally throttled.

Off-host backup copying is a manual step the owner has to automate; nothing in this repository does it.

`linux/amd64` is unverified, as described under Architectures.
