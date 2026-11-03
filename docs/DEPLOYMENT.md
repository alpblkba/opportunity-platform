# Deployment architecture

Opportunity Platform runs as a single-owner service on a Linux host with Docker Engine and the Compose plugin. The supported GCP release path uses prebuilt Linux amd64 images, PostgreSQL and Cloudflare Tunnel. Use [RELEASE_OPERATIONS.md](RELEASE_OPERATIONS.md) for production releases, backups, restore rehearsals and recovery, and [AUTHENTICATION.md](AUTHENTICATION.md) for identity configuration and cutover.

## Services and persistent state

| Service | Responsibility | Persistent state |
| --- | --- | --- |
| `app` | API, built React interface, source scheduler and application queue | `/app/private`, mounted from the `private` volume |
| `db` | PostgreSQL data, source history, profiles, sessions and application state | `/var/lib/postgresql/data`, mounted from `db-data` |
| `tunnel` | Outbound connection to Cloudflare, forwarding the configured hostname to `http://app:8787` | Tunnel configuration managed in Cloudflare |

The application uses one replica. Browser assistance, when installed, runs as a child process per task rather than a separate service. The standard production release target does not include that worker.

Neither Compose file publishes host ports. Services share the `internal` network, which is a Docker bridge network; its name does not imply `internal: true` or prevent outbound connections. Source discovery and the tunnel require outbound connectivity. Keep the application and database inaccessible through public host ports.

Cloudflare Access protects the public hostname in `cloudflare` mode, and the backend independently validates Access assertions. Native modes have different boundaries described in [authentication modes](AUTHENTICATION.md#authentication-modes). `/health` reports liveness and package version; `/ready` also checks a database query. Neither identifies the reviewed Git revision. The release scripts compare immutable image IDs and `org.opencontainers.image.revision`; authenticated `/api/ops/status` also reports application operational state.

## Compose profiles and capacity

The base [docker-compose.prod.yml](../docker-compose.prod.yml) uses `pgvector/pgvector:pg16` with memory limits of 1 GiB each for the app and database and 256 MiB for the tunnel. The [e2-micro overlay](../docker-compose.e2micro.yml) selects `postgres:16-alpine`, limits the app/database/tunnel to 256/192/64 MiB, and uses a locally loaded `APP_IMAGE` with `pull_policy: never`.

The overlay sets PostgreSQL `shared_buffers=64MB`, `work_mem=2MB`, `maintenance_work_mem=32MB`, `max_connections=12`, `max_parallel_workers_per_gather=0`, `jit=off`, `checkpoint_timeout=15min` and `max_wal_size=512MB`. Migrations use `pgcrypto`; embeddings are stored in `real[]` and do not require the pgvector extension.

Run Compose from `/opt/opportunity` with both files and `.env.prod` for this profile:

```bash
docker compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod ps
docker compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod logs --tail 50 app
```

The 512 MiB combined container limits are configuration, not a measurement of total host usage. Linux, Docker, filesystem cache and transient allocations need additional memory. Deployment preflight requires at least 3 GB free on the root filesystem and reports available memory; that check does not prove workload capacity. Monitor disk consumption from retained images, database growth and private snapshots.

Historical local testing reported Linux amd64 runtime checks under Apple Silicon emulation. Native amd64 capacity on a representative 1 GiB host remains unverified by that evidence. Earlier local browser tests reported failures at the 256 MiB application limit and successful mock-assisted runs with a 448 MiB limit, but those results do not establish production browser capacity. The supported e2-micro release remains browser-free.

### Image targets

[Dockerfile](../Dockerfile) has `runtime` and `browser` targets. `runtime` includes the Rust binary, built frontend, configuration, CA certificates, curl and `pdftotext`; it runs as UID 10001. `browser` additionally includes Node, the compiled worker and Chromium installed by the worker's Playwright package.

Always select a build target explicitly. The final Dockerfile stage is `browser`, so a bare Docker build, including the base Compose build configuration, selects that stage despite the Dockerfile comment describing `runtime` as the default. The production builder [release-image.sh](../scripts/release-image.sh) explicitly requires `--target runtime` and `linux/amd64`. The browser target does not carry the same revision-label configuration and is not supported by that release builder.

Build on a separate machine. Do not compile Rust or the frontend on the e2-micro. Build and release commands are in the [immutable image procedure](RELEASE_OPERATIONS.md#immutable-images).

## Runtime configuration

Create `.env.prod` from [.env.prod.example](../.env.prod.example) on the host, set mode 600, and keep it out of Git and release archives. The release scripts preserve host secrets and update `APP_IMAGE` after loading the reviewed archive.

| Setting | Behavior |
| --- | --- |
| `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB` | Initialize PostgreSQL and construct the app's `DATABASE_URL`; database name defaults to `opportunity` |
| `APP_IMAGE` | Image loaded on the VM; production releases use an immutable revision tag |
| `ACCESS_MODE` | Defaults to `cloudflare` in Compose |
| `CF_ACCESS_TEAM_DOMAIN`, `CF_ACCESS_AUD`, `ACCESS_ALLOWED_EMAILS` | Required for `cloudflare` and `both`; empty allowlists are refused |
| `CLOUDFLARE_TUNNEL_TOKEN` | Required tunnel credential |
| `DATABASE_MAX_CONNECTIONS` | e2-micro overlay default: 4 |
| `MATCH_SHORTLIST` | e2-micro overlay default: 2,000 candidates |
| `EMBEDDINGS_PROVIDER` | Defaults to `disabled` |
| `EMBEDDINGS_ENDPOINT`, `EMBEDDINGS_MODEL` | Provider configuration when embeddings are enabled |
| `RUST_LOG` | Defaults to `info` |

Compose sets `API_HOST=0.0.0.0` and `API_PORT=8787`. The runtime image sets `WEB_DIST=/app/apps/web/dist`, `RAW_SNAPSHOT_DIR=/app/private/cache` and `APP_REVISION` from the build revision. The bind-address guard refuses `ACCESS_MODE=loopback` on `0.0.0.0`.

Generate a URL-safe database password, for example with `openssl rand -hex 24`. Compose interpolates it into a PostgreSQL URL without percent-encoding. Changing `POSTGRES_PASSWORD` in `.env.prod` does not change the password in an existing database volume; update the PostgreSQL role first as part of an authorized credential rotation, then update the file.

The executable supports settings that Compose does not pass through, including `SECURE_COOKIES` and `BROWSER_WORKER_ALLOW_EXTERNAL`. Adding a value to `.env.prod` alone does not expose it inside the container. Review Compose configuration before relying on an additional runtime setting. Avoid printing resolved Compose configuration or the full container environment because they contain credentials.

Semantic reranking requires a reachable provider; the owner's laptop is not a production dependency. The separate profile-matching API can use embeddings, while dashboard `career-priority-v3` remains lexical. See [RANKING_POLICY.md](RANKING_POLICY.md). Keep embeddings disabled until the provider and resource budget have been configured and verified.

## GCP infrastructure and prerequisites

The deployment scripts default to project `project-484f9dc3-1a36-4b55-9e8`, instance `opportunity-platform`, zone `us-central1-a`, and directory `/opt/opportunity`. `GCP_PROJECT`, `GCP_INSTANCE`, `GCP_ZONE` and `REMOTE_DIR` override those values. These defaults identify the intended deployment; they do not confirm current infrastructure state.

The VM needs Docker Engine, the Compose plugin, Bash, GNU `timeout`, `flock` and `sha256sum`. The deployment user must own `/opt/opportunity`, read `.env.prod`, and access Docker directly or through passwordless sudo. The operator machine needs Git, Docker Buildx, Python 3, the Google Cloud CLI, and the GitHub CLI; backup and restore procedures also require GnuPG and Docker.

IAP carries administrative SSH and file transfer to a VM without a public address. It still requires an ingress firewall rule allowing Google's IAP range `35.235.240.0/20` to TCP 22, IAM authorization and OS Login. No public application ingress rule is needed. [RELEASE_OPERATIONS.md](RELEASE_OPERATIONS.md#12-iam-iap-and-os-login) defines the service-account permissions and their verification.

For Cloudflare, configure a tunnel hostname pointing to `http://app:8787`, an Access application for that hostname and an owner-only identity policy. The documented hostname is `opportunities.alpblkba.dev`. Complete identity and tunnel configuration before starting the application. Access policy changes require separate owner authorization.

The scripts can also use ordinary SSH through `TRANSPORT=ssh` and `SSH_HOST`; this does not provision another provider or its network controls. Provider prices, promotional credits and instance recommendations are outside the supported release procedure.

## Release lifecycle

An operator reviews a full commit SHA and builds a clean checkout using `release-image.sh`. The builder packages only tracked files from that commit, checks the image and writes an archive plus SHA-256 checksum. `deploy-vm.sh` checks the candidate against the running image revision, configured image and actual SQLx migration history, then transfers and verifies the artifact.

The VM release helper records a durable journal, retains the previous immutable image and configuration, loads the candidate, recreates only `app`, and verifies readiness, identity configuration and anonymous access behavior. A failed code-only release attempts verified recovery. An uncertain transport result or unprovable recovery requires explicit reconciliation; it must not be treated as success or retried blindly.

SQLx migrations run at application startup. A new migration always requires a manual rollout, explicit owner authorization and a fresh, restore-verified recovery point. Existing applied migrations must remain byte-for-byte compatible with their stored SHA-384 checksums. An older image can refuse a newer migration history, so image rollback cannot reverse a schema upgrade.

The deployment workflow runs only when `DEPLOY_ENABLED=true`. It validates successful CI for the exact current `master` SHA, obtains short-lived GCP credentials through Workload Identity Federation, and deploys code-only changes. Manual workflow dispatch does not bypass the migration gate. Environment review requirements, branch protection, identity setup and revocation are documented in [RELEASE_OPERATIONS.md](RELEASE_OPERATIONS.md).

`--preflight` performs inspection rather than deployment. `--check-transfer` writes and removes an empty transfer probe. Status and gate operations do not replace the running release, although the VM helper opens its lock file. Treat these distinctions as operational behavior, not a guarantee that no local tool or lock state is written.

### Initial data transfer

[migrate-to-cloud.sh](../scripts/migrate-to-cloud.sh) exports a local database, restores it into the production Compose stack selected by the local Docker context and compares table counts. It invokes `pg_restore --clean --if-exists`, which can replace existing production objects. It is an initial data-transfer utility, not the supported upgrade path for an existing production database. Its historical local rehearsal does not authorize an overwrite or establish current compatibility. Use the release and recovery procedures for the existing VM.

## Backups and retention

The documented operating policy is to pull encrypted backups to the owner's Mac. `deploy-vm.sh --fetch-backup` streams a custom-format PostgreSQL dump and `/app/private` over IAP, encrypts them to a pinned OpenPGP fingerprint on the receiving machine, and records checksums and production metadata in a manifest. The VM does not store backup archives or storage credentials.

A database dump alone is insufficient: CVs and browser evidence live in the private volume. The dump precedes the private-tree archive, and the restore rehearsal checks every referenced document against its recorded content hash. Backup streams are not a fully atomic snapshot of all application activity. A restore-verified backup matching the current production revision and migration history, created less than 24 hours earlier, is required for schema changes.

Backups exist only when a backup operation runs. The optional [backup workflow](../.github/workflows/backup.yml) is gated by `BACKUP_ENABLED=true`; its schedule is 03:17 UTC daily. Enabling it requires separate bucket, key and identity configuration. See the [backup procedures](RELEASE_OPERATIONS.md#backups-and-restore-rehearsals) for manual backups, restoration, optional automation and retention.

## Background work and application submission

Source cadence, failures and next-run times persist in PostgreSQL. A scheduler tick checks due sources every minute. A PostgreSQL advisory lock serializes refresh sweeps and browser work; a refresh skips when the lock is held, while a browser operation waits for a bounded interval. The lock uses a connection detached from the pool and is released when that connection closes. The one-replica configuration limits duplicate background loops.

The runtime release has no browser worker, so assisted submission falls back to manual handling. A browser-capable deployment additionally requires an explicit external-access setting, `BROWSER_WORKER_ALLOW_EXTERNAL=owner-supervised`, for live employer pages. Automatic submission also requires an enabled owner-created policy and current eligibility checks. Neither a successful deployment nor a ranking score authorizes a live submission.

## Maintenance boundaries

Production state and capacity must be checked on the actual host; repository tests and historical local measurements are insufficient evidence. Preserve `.env.prod`, both persistent volumes, release journals and rollback images needed for recovery. Do not use database migration-history edits to force an incompatible binary to start.

The tunnel image is pinned to `cloudflare/cloudflared:2026.10.3` by digest in Compose. App-only releases do not recreate the tunnel or database; updating those services is a separate maintenance operation. Keep their configuration and recovery requirements under review rather than assuming an app deployment updates every service.
