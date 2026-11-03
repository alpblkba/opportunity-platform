# Authentication and authorization

Opportunity Platform is a single-owner service. Authentication protects the CV, application answers, documents and application workflow. The backend enforces access to private API routes; frontend routing is not an authorization boundary.

The implementation supports Cloudflare Access and native password sessions. The previous deployment record dated 2026-10-27 reported revision `1043ab9` in `ACCESS_MODE=cloudflare`. That is historical evidence, not a current production status check. Native authentication has local test coverage, but its production cutover is not established by the repository. Check the running configuration using the [release runbook](RELEASE_OPERATIONS.md) before changing modes.

## Authentication modes

| `ACCESS_MODE` | Private API requirement | Operational use |
| --- | --- | --- |
| `loopback` | No credential; requests resolve to the local owner | Development only; binding is restricted to `127.0.0.1`, `::1` or `localhost` |
| `cloudflare` | Valid, allowlisted Cloudflare assertion | Access-protected deployment; native cookies do not grant access |
| `both` | Valid, allowlisted Cloudflare assertion and valid native session | Native-login verification while Access remains enforced |
| `native` | Valid native session | Application authentication after a separately authorized Access policy change |

The executable defaults to `loopback`; production Compose defaults to `cloudflare`. Unknown modes fail startup. `guard_bind_address` refuses a non-loopback bind in `loopback` mode. The production Compose files publish no ports. See [deployment architecture](DEPLOYMENT.md).

`cloudflare` and `both` require `CF_ACCESS_TEAM_DOMAIN`, `CF_ACCESS_AUD` and a nonempty comma-separated `ACCESS_ALLOWED_EMAILS`. Keep the allowlist limited to identities authorized to access the same owner's data. Native mode does not use these variables for identity validation, but deployment preflight still requires their values in `.env.prod`.

## Cloudflare Access

[access.rs](../crates/api/src/access.rs) reads `cf-access-jwt-assertion`, falling back to the `CF_Authorization` cookie. It validates the RS256 signature, key ID, audience, issuer and expiry, then checks the normalized email against the allowlist. An unverified email header never establishes identity.

Public keys come from `https://<CF_ACCESS_TEAM_DOMAIN>/cdn-cgi/access/certs` and are cached for 30 minutes. Missing or invalid assertions and disallowed identities return 401 JSON. A key-fetch failure returns 503. A native cookie alone cannot authenticate in `cloudflare` mode. In `both`, the assertion and session must each pass; the implementation does not require their email addresses to match.

Cloudflare Access, Tunnel and application authentication have separate roles. The tunnel transports traffic to the private origin; an Access policy controls admission at Cloudflare; the application validates the configured credentials. Publishing a tunnel hostname does not itself configure an Access policy.

## Routes

The [router](../crates/api/src/routes.rs) applies identity middleware to private API routes, then merges the three authentication routes outside that middleware.

| Route | Behavior |
| --- | --- |
| `GET /health` | Public at the origin; liveness and package version |
| `GET /ready` | Public at the origin; also checks database connectivity |
| `POST /api/auth/login` | Password login; Origin check and login limits apply in every mode |
| `POST /api/auth/logout` | Clears the native cookie and attempts to revoke its session; Origin check applies |
| `GET /api/auth/session` | Resolves identity using the configured mode; returns identity or an authentication error |
| Other registered `/api` routes | Require the configured identity, including `/api/ops/status` |
| Frontend assets and fallback pages | Public at the origin in `both` and `native`; identity middleware applies in `cloudflare` |

Cloudflare may still require authentication before a request reaches a route that is public at the origin. Login can issue a native cookie in `cloudflare` mode, but that cookie is ignored for API authorization. Native logout does not end a Cloudflare Access session. Logout clears the browser cookie even if database revocation fails, so its response alone does not prove server-side revocation succeeded.

The frontend serves the landing page at `/` and `/index.html`, the login form at `/login`, and the workspace at other fallback paths, including `/workspace` and `/matches`. It checks `/api/auth/session` before rendering private workspace content. The bundle contains no personal data; private content is retrieved through the API. Unknown frontend paths resolve to the workspace rather than a dedicated not-found page.

## Accounts and sessions

Accounts are stored in `users` and refer to `owners`. Only one `owner` account is supported, enforced by the `users_single_owner` index in [0005_native_authentication.sql](../migrations/0005_native_authentication.sql). There is no registration endpoint, default password or email password-reset flow.

Passwords use Argon2id with 19 MiB of memory, two passes and one lane, stored as PHC strings. Passwords must contain at least 12 characters and at most 1,024 bytes. Password hashing and verification use blocking tasks guarded by a two-permit semaphore. Unknown accounts use a lazily initialized decoy hash for verification. Unknown addresses, incorrect passwords and disabled accounts return the same login refusal; this is not a guarantee of identical timing for every error path.

Session tokens contain 256 bits of operating-system randomness and are hex encoded. Only their SHA-256 hashes are stored in `user_sessions`. Sessions expire after 12 hours. Each lookup checks expiry, revocation, account disablement and owner role; there is no device binding. Password changes revoke the account's sessions. The scheduler purges sessions expired or revoked for more than seven days every 60 scheduler ticks; expiry enforcement does not depend on cleanup.

### Cookies and request origin

Non-loopback bindings default to `__Host-opportunity_session` with `Secure`, `Path=/`, `HttpOnly`, `SameSite=Lax` and no `Domain`. Loopback bindings default to `opportunity_session` without `Secure`. `SECURE_COOKIES=yes` forces the secure cookie for a loopback server behind HTTPS; `SECURE_COOKIES=no` disables it. Production must retain secure cookies. This override is read by the executable and is not passed through by the supplied Compose files.

The Origin check applies to methods other than GET, HEAD and OPTIONS, including login and logout. When `Origin` is present, the portion after `://` must equal `Host`. Missing Origin headers are accepted; the check does not validate the scheme separately. `SameSite=Lax` provides an additional browser-side restriction. See [auth.rs](../crates/api/src/auth.rs) for the exact behavior.

### Login limits and errors

Login failures are tracked by normalized account email, not client IP. Five failures cause a 60-second lockout; subsequent failures double the delay up to 15 minutes. The in-memory map holds at most 512 addresses with eviction and is cleared on restart. Successful login clears the address's failure state. At most eight login requests may be in flight.

Lockouts return 429 with `Retry-After`; login-capacity exhaustion returns 503 with `Retry-After: 5`. Origin failures return 403. Session creation failures return 500 without database details. API authentication failures remain JSON responses rather than HTML redirects.

These limits do not prevent repeated denial of login to a known account. Configure and test an edge rate limit for `/api/auth/login` before exposing native login publicly. Native authentication has no implemented TOTP or hardware-key second factor. A stolen session remains usable until expiry or successful revocation.

## Account administration

Run account commands on the VM from `/opt/opportunity`, using the deployed image and database. Create the account while `ACCESS_MODE=cloudflare` remains active, before switching to `both` or `native`.

```bash
sudo docker compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod \
  exec app opportunity user create --email you@example.com --name "Alp"
```

The interactive prompt reads the password twice without echo. A password manager can supply it through standard input; `-T` disables the container TTY. Keep passwords out of command arguments and logs.

```bash
pass show opportunity/owner | sudo docker compose \
  -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod \
  exec -T app opportunity user create --email you@example.com --name "Alp"

sudo docker compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod \
  exec -T app opportunity user list
sudo docker compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod \
  exec app opportunity user passwd --email you@example.com
sudo docker compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod \
  exec app opportunity user revoke-sessions --email you@example.com
```

`user list` does not expose password hashes or session tokens. Password recovery requires host and database access through `user passwd`; there is no web recovery mechanism. Loss of both password and host access requires infrastructure recovery and, if necessary, restoration from a verified backup.

## Authorization boundaries

The application does not implement tenant isolation. Personal-data queries are not consistently scoped by the authenticated `owner_id`; many operate on the first owner or global tables. Adding a second account could expose the existing owner's data. Do not enable additional users by inserting rows directly.

The schema accepts `user` and `agent` role values, but account creation, password changes, authentication and session resolution reject non-owner accounts. Agent authentication and multi-user access are not implemented. Enabling them requires owner scoping across reads, writes, joins, exports, notifications, background jobs and application preparation, with negative cross-owner tests.

Authentication does not authorize employer submission. Submission also requires the application workflow's authorization checks, eligibility assessment and worker access policy. See [ranking and automatic applications](RANKING_POLICY.md#automatic-applications).

## Native authentication cutover

A cutover requires explicit owner authorization separate from an application release. Keep Access enforced throughout schema changes and release recovery. Deploy and verify the candidate in `cloudflare` mode using [RELEASE_OPERATIONS.md](RELEASE_OPERATIONS.md); do not assume only migration `0005` is pending.

Bypassing Access only for the login page and assets leaves private API requests subject to Access login, which prevents a native-only session from loading the workspace. Cloudflare evaluates Bypass and Service Auth before Allow and Block; a Bypass policy disables Access enforcement and request logging. Path scope belongs to the Access application's destination. See [Cloudflare Access policies](https://developers.cloudflare.com/cloudflare-one/access-controls/policies/).

If authorized, retain the Access application and its audience tag so the owner-only Allow policy can be restored. Deleting the application requires recreating that configuration for rollback. Tunnel routing is a separate configuration and need not change.

1. Create and list the usable owner account while Access remains enforced.
2. Set `ACCESS_MODE=both`, recreate the application container, run `vm-check.sh`, and verify both Access login and native login.
3. Set `ACCESS_MODE=native`, recreate the application container, and run `vm-check.sh` again while Access is still enforced. Confirm the checker reports that the container and `.env.prod` both use `native` and that unauthenticated private API requests are refused.
4. After separately approving public native login and configuring its edge rate limit, replace the owner-only Allow policy with a single Bypass/Everyone policy for the hostname. Keeping an Allow policy alongside Bypass does not preserve enforcement.
5. Complete the acceptance checks below. Restore the previous policy and mode if they fail.

For stages 2 and 3, run on the VM from `/opt/opportunity`:

```bash
sed -i 's|^ACCESS_MODE=.*|ACCESS_MODE=both|' .env.prod
sudo docker compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod up -d --no-build app
sh vm-check.sh

# Proceed only after both authentication steps pass.
sed -i 's|^ACCESS_MODE=.*|ACCESS_MODE=native|' .env.prod
sudo docker compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod up -d --no-build app
sh vm-check.sh
```

Editing `.env.prod` does not change a running container. `vm-check.sh` checks the actual container mode, usable owner account and anonymous HTTP behavior. Both `both` and `native` return 401 for anonymous private API requests, so HTTP status alone cannot distinguish them. Bypassing Access while the container still requires Cloudflare assertions causes lockout.

### Acceptance and rollback

Use a fresh private browser window after the policy change:

1. `/login` renders the application login form without a Cloudflare login screen.
2. `/api/stats` returns 401 JSON before native login.
3. Incorrect passwords and unknown addresses produce the same refusal.
4. Correct credentials load the workspace and authenticated statistics.
5. Reload and a `/matches` deep link retain the session.
6. Logout followed by browser Back shows the login form without private content.
7. A second device without a session receives 401 from `/api/applications`.

After Bypass has been enabled, restore the owner-only Allow policy before reverting to `ACCESS_MODE=cloudflare` and recreating the application. Before the policy change, mode rollback can occur with Access left in place. Image rollback does not change Cloudflare policy or reverse SQLx migrations; database recovery requires a compatible image, verified recovery point and separate owner authorization as described in the [release runbook](RELEASE_OPERATIONS.md#rollback-and-recovery).

## Verification scope

[access_assertions.rs](../crates/api/tests/access_assertions.rs) exercises signed test assertions and invalid credentials. [native_auth.rs](../crates/api/tests/native_auth.rs) covers native sessions and authentication boundaries; frontend tests cover signed-out routing. These tests do not establish current Cloudflare configuration, a completed production cutover or production capacity. The historical local rehearsals must not be treated as production acceptance.
