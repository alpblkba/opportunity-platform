# Who gets in

This service holds one person's CV, their answers to application questions, and the authority to submit applications on their behalf. Everything below exists to keep that to one person.

**Status: native authentication is implemented and tested locally. Not deployed.** Production still runs the earlier application with `ACCESS_MODE=cloudflare`. The native-authentication deployment, account creation and cutover described below have not been executed there.

## What production runs today

Cloudflare Access is the trust boundary. A tunnel brings traffic to an origin with no public address; Access decides who reaches it; the application validates the assertion itself rather than trusting the tunnel. That validation checks the RS256 signature against the team's published keys, the audience tag, the issuer and the expiry, and then the identity against an allowlist that may not be empty.

It works and it stays. The weakness is not security, it is shape: there is no account, so the application cannot tell one person from another, cannot sign anyone out, and cannot offer a login page of its own. Someone arriving at `alpblkba.dev` meets Cloudflare's one-time-code screen rather than anything belonging to this project.

## What was added

An account, a password, and a session.

**Accounts** live in `users`. Each belongs to an `owners` row, which is what the data belongs to. A role says what kind of account it is: `owner` for the person this exists for, `agent` for a non-interactive client, `user` for an additional human. Only one `owner` account can be created today; `agent` and `user` accounts are refused for the reason in "Multi-user" below.

**Passwords** are Argon2id, 19 MiB and two passes, stored as PHC strings. Nothing reversible, nothing with a default. Hashing happens off the async runtime behind a semaphore of two, because each hash allocates 19 MiB and an unbounded number of simultaneous logins would be a memory exhaustion attack that needs no credentials. An unknown address is charged the same verification as a known one, against a hash of a password nobody has, so the clock says nothing about whether an account exists.

**Sessions** are 256 bits of operating-system randomness. The database stores only their SHA-256, so a copy of the table is not a set of working cookies. They expire after twelve hours, can be revoked individually or per account, and are checked on every request with expiry and revocation in the query rather than after it.

## Routes and what they require

| Route | Who may call it |
|---|---|
| `GET /health`, `/ready` | anyone; status and version only |
| `GET /` | anyone, in native modes: the public landing page |
| `GET /login` | anyone, in native modes. It is the single-page app, which contains no personal data |
| `GET /workspace` | anyone, in native modes, and it shows nothing: the application asks `/api/auth/session` first and renders the login page when the answer is no |
| `POST /api/auth/login` | anyone, rate limited, Origin checked |
| `POST /api/auth/logout` | anyone; revokes the session it carries, if any |
| `GET /api/auth/session` | anyone; 200 with who you are, 401 with nothing |
| everything else under `/api` | an authenticated identity, by whichever mode is configured |
| the rest of the interface | in native modes, served publicly; every answer it shows comes from `/api` |

The three public auth routes are merged into the router *after* the identity middleware is applied, rather than excepted from it. There is no list of exemptions to get wrong.

The single-page app owns `/`, `/login` and `/workspace`, and the same bundle answers all three. `/` and `/index.html` are the landing, `/login` is the form, and **every other path is the workspace** — there is no not-found page, so a mistyped deep link resolves to the application and then to the login form if nobody is signed in. Each of those is a real URL that survives a reload and the back button, which the frontend tests exercise by setting the path before rendering. That is safe for exactly one reason: the bundle contains no personal data. Every piece of it comes from `/api`, which is behind the gate in every mode, so a stranger who loads `/workspace` gets markup and JavaScript and then a 401. In `cloudflare` mode the bundle itself also stays behind Access, unchanged, which is what keeps the rollback honest.

## The four modes

`ACCESS_MODE` decides the boundary. It is the one setting that matters here.

| Mode | Cloudflare assertion | Native session | Use |
|---|---|---|---|
| `loopback` | — | — | local development; refuses to bind anything but 127.0.0.1 |
| `cloudflare` | required | ignored | **production today**, and the rollback |
| `both` | required | required | the cutover rung: native login exercised in production with nothing opened |
| `native` | — | required | after the cutover |

`both` is the point of the ladder. In it, Cloudflare still decides who reaches the origin and the application additionally demands a session, so the owner signs in twice — once with the one-time code, once with the password. Nothing is opened, and the native path is proven in the place it will actually run. A test asserts the converse: in `cloudflare` mode a session cookie opens nothing, so the rollback is exactly what it was.

## The cookie

`__Host-opportunity_session` where the service is reached over HTTPS, `opportunity_session` on a loopback development server, decided by the bind address. The `__Host-` prefix is a promise the browser enforces: `Secure`, `Path=/`, and no `Domain`, so the cookie belongs to that one hostname and a sibling subdomain cannot set it. `HttpOnly`, so no script reads it. `SameSite=Lax`, so a cross-site POST does not carry it while a top-level link from `alpblkba.dev` still does.

Cross-site requests are refused twice. `SameSite=Lax` is the browser's lock; an `Origin` check on every unsafe method is this service's, and it does not depend on the browser having got the first one right. Login itself is checked the same way, because a form posted from another site is login CSRF.

## Errors

The API returns real statuses: 401 when nobody is authenticated, 403 when somebody is and may not do this, 429 with `Retry-After` when an address has been guessed at too often, 500 when something broke. JSON failures stay JSON; nothing redirects an API call to an HTML page.

A wrong password and an unknown address produce the same body, byte for byte, which a test asserts. Internal errors log their detail and tell the caller only that something failed — a database error carries constraint names, column names and sometimes fragments of a connection string, and none of that belongs in a browser.

The playful error pages on `alpblkba.dev/401/` and friends belong to the website. This service links to them from the login page rather than recreating them, and never redirects an API response to one.

## Starting the first account

There is no registration page and no default password. The first account is made on the machine:

```bash
# on the VM, in /opt/opportunity
sudo docker compose -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod \
  exec app opportunity user create --email you@example.com --name "Alp"
# it prompts twice, echoes nothing, and the password never reaches argv or the shell history
```

A password can also be piped in, which is how a password manager supplies one:

```bash
pass show opportunity/owner | sudo docker compose ... exec -T app opportunity user create --email you@example.com
```

Related commands: `user list` (no hashes, no secrets), `user passwd --email ...` (which also ends every session that account had), `user revoke-sessions --email ...`.

Recovery is the same path: anyone who can reach the machine can set a new password. That is deliberate and it is the whole recovery story — there is no reset email, no security question and no permanent backdoor. Losing both the password and access to the machine means restoring from a backup.

## Multi-user: not yet, and why

`owner_id` exists on eleven tables. **No query filters by it.** The value is written on insert, always taken from the first `owners` row, and never read back as a condition. A second account today would read and write the first one's CV, facts, applications and policies.

So the schema refuses to have one. A unique index permits at most one `owner` account, and `user` accounts are refused with that reason. A test asserts both refusals, and asserts that no registration route exists.

What enabling additional accounts requires, concretely:

1. An owner argument through the personal-data modules — 12 public functions in `profiles.rs`, 27 in `applications.rs`, 4 in `digest.rs` — and a `where owner_id = $n` on every read, update and delete, including lookups by record id.
2. The same for indirect paths: search and filtering, joins that reach personal rows, export, the digest generator, the review queue, notifications, and anything the application-preparation workflow touches.
3. Background work decided per owner rather than globally.
4. Negative tests: a second account reading, updating and deleting the first one's records by id, and the first one's data never appearing in the second one's lists or exports.

Until that is done, this is a secure single-owner application with an architecture ready for more, and it says so rather than implying otherwise.

**Only an owner account can be created at all.** `agent` and `user` are values the `role` column accepts and the application refuses: `create_account` rejects anything but `owner`, `set_password` refuses a non-owner row, `authenticate` refuses one, and `session_account` refuses a session belonging to one — four places, because a session outlives the login that made it. A test inserts an `agent` row directly, carrying the owner's own password hash, and confirms it can neither log in nor use a session minted for it. Agent authentication is therefore **not implemented**; the enum variant is a placeholder for the work described above, not a feature.

## Resource cost

Measured on the development machine; the production budget is 256 MiB for the application.

| | |
|---|---|
| Argon2id, one hash | 19 MiB, about 60 ms |
| Concurrent hashes permitted | 2, so 38 MiB at the worst moment |
| Session lookup | one indexed query |
| Login attempt tracking | an in-memory map, hard-capped at 512 addresses with eviction |
| Logins in flight | capped at 8, so a flood is refused rather than queued behind the hashing permits |
| New dependency | `argon2` only; `rpassword` for the CLI prompt |
| Frontend | 275 kB from 272 kB |

No Redis, no second service, no separate timer. `purge_expired_sessions` runs on the refresh loop that already exists, once every sixty one-minute ticks, and deletes sessions that expired or were revoked more than seven days ago. Expiry itself is enforced in the lookup, so the sweep is housekeeping rather than a control: if it never ran, old rows would accumulate and nothing else would change.

## The Cloudflare cutover

Adding a login page does not make it reachable. Cloudflare Access protects the whole hostname, so `https://opportunities.alpblkba.dev/login` meets the one-time-code screen first. The cutover is a Cloudflare change, not a code change, and it has not been made.

### Why path-scoped Bypass is the wrong shape

An earlier draft of this document proposed bypassing Access for `/login` and the static assets while leaving `/api/*` protected. That does not work, and it is worth being precise about why: the login page would load, the password would be accepted, and then the first `fetch('/api/stats')` would be intercepted by Access and answered with a redirect to the one-time-code screen. An XHR cannot complete an interactive login, so the workspace would simply fail to load. Half-protecting the hostname protects nothing and breaks everything.

Three facts decide the shape, from [Cloudflare's policy documentation](https://developers.cloudflare.com/cloudflare-one/policies/access/):

- Access evaluates **Bypass and Service Auth first**, then Allow and Block in order, and stops at the first Allow or Block match. A Bypass policy therefore wins over an Allow policy on the same application: you cannot keep an owner-only Allow "as a safety net" behind a Bypass and expect it to protect anything.
- Bypass is scoped by the **application's destination**, not by a path field inside the policy. Path scoping means a second Access application whose destination is that path.
- Bypass **disables enforcement and does not log**. It is an absence of Access, not a quieter Access.

Access, Tunnel and WAF are three separate things, and only the first is being removed. The tunnel keeps transporting traffic to an origin with no public port; WAF rules and rate limiting keep applying, because they are evaluated by the proxy regardless of Access.

### The shape that works

Access stops enforcing on this hostname, and the application becomes the boundary.

Two ways to arrange that, and they differ less than they look. **Replacing the owner-only Allow policy with a single Bypass/Everyone policy** keeps the application object, and with it the audience tag and a one-change path back. **Deleting the Access application** removes the object entirely, which means rebuilding it to roll back, and means the audience tag changes so `.env.prod` has to change with it. Bypass is the better of the two for exactly that reason: the rollback is a policy swap rather than a reconstruction. Both leave the tunnel untouched, and neither opens a port on the VM. That is coherent precisely because the application already checks identity on every `/api` route in the backend, returns 401 to anything unauthenticated, and serves an interface shell that contains no personal data.

The public landing page at `/` is part of what this opens: it is served to anyone in `native` mode and is meant to be, which is why the shell carries no personal data and every answer comes from `/api`.

The order below is not a suggestion. `vm-check.sh` fails a deployment in `both` or `native` mode when no account exists, which is correct: a login page nobody can get past is a broken release, so the account has to exist before the mode changes. This order was rehearsed on a local 1 GiB VM, not the production GCP instance.

**Step 0 — deploy the new image, still in `cloudflare` mode.** Follow the owner's separately approved production rollout procedure only after its release gates close. Migration 0005 adds tables without changing existing records, but the older binary rejects the newer SQLx migration history; image rollback alone cannot recover this upgrade.

```bash
./scripts/deploy-vm.sh dist/opportunity-app-<tag>-amd64.tar.gz     # .env.prod still says cloudflare
```

Verify: the log says `access: Cloudflare Access`, and the database reports five migrations with a `users` table present.

**Step 1 — create the owner account, on the machine.** The password is piped so it never reaches `argv`, the process list, the shell history or a log. `-T` is what makes the pipe work.

```bash
# on the VM, in /opt/opportunity. Read the password from a manager, or type it at a prompt by
# dropping the pipe and the -T.
pass show opportunity/owner | sudo docker compose \
  -f docker-compose.prod.yml -f docker-compose.e2micro.yml --env-file .env.prod \
  exec -T app opportunity user create --email you@example.com --name "Alp"

# check: one account, and what is stored is a hash
sudo docker compose ... exec -T app opportunity user list
```

Typing it instead: drop `-T` and the pipe, and the CLI prompts twice with no echo. Either way the only place the password exists afterwards is the Argon2id hash in `users.password_hash`.

**Step 2 — prove it with nothing opened.** Set `ACCESS_MODE=both` in `.env.prod` and restart the application container. No Cloudflare change at all: Access still guards the hostname, and the application additionally demands a session. Sign in with the one-time code, then with the password.

```bash
sed -i 's|^ACCESS_MODE=.*|ACCESS_MODE=both|' .env.prod
sudo docker compose ... up -d --no-build app
sh vm-check.sh          # must pass, and will refuse if no account exists
```

**Step 3 — make the application the boundary.** Set `ACCESS_MODE=native` the same way. Still no Cloudflare change: Access is now asking for a code the application no longer needs, so the experience is an extra screen and nothing is weaker. `/`, `/login` and `/workspace` begin serving publicly at the origin while `/api/*` still answers 401, which `vm-check.sh` asserts. If a check fails, restore the previous access mode and recreate app while Access remains active.

**Before step 4, confirm what is actually running.** `vm-check.sh` compares the `ACCESS_MODE` in the application container's own environment against the one in `.env.prod` and fails if they differ, because editing the file without restarting leaves the old mode serving. In `both` and `native` the unauthenticated status codes are identical, so the file alone cannot tell them apart — and the difference between them is whether Cloudflare Access is still required. Switching Access to Bypass while the application is still in `both` locks everyone out; doing it while the file *says* `native` and the container is still on `cloudflare` opens the hostname with no native gate behind it. This check is the thing that catches both, and it is mandatory before the policy changes.

```bash
sh vm-check.sh        # must print: the container is running ACCESS_MODE=native, which is what .env.prod says
```

**Step 4 — separate owner-authorized policy change, outside the release runbook.** In Zero Trust, on the application covering `opportunities.alpblkba.dev`, replace the owner-only Allow policy with a single **Bypass / Everyone** policy. Do not keep both: Bypass is evaluated first and the Allow would never be reached, so leaving it would be a comforting no-op.

Keep the application object itself rather than deleting it. It holds the audience tag, and putting the Allow policy back is then one change rather than a rebuild.

**After step 4 the hostname is open to the internet as far as Cloudflare Access is concerned.** What stands between a stranger and the CV is this application: a 401 on private `/api` routes without a session, Argon2id on the login endpoint, a bounded lockout, and an Origin check. That is the trade being made deliberately, and it is why the login endpoint's own defences matter more after the cutover than before.

**Recommended before step 4,** given a 1 GiB machine that can hash two passwords at a time: a Cloudflare rate-limiting rule on `/api/auth/login`, something like 10 requests per minute per IP with a managed challenge above it. It costs nothing on the origin, it absorbs the flood this machine cannot, and it is the one protection that survives Access being bypassed. A WAF custom rule blocking obvious scanners on `/api/*` is cheap too.

### Rollback

| What went wrong | What to do |
|---|---|
| Step 4 opened too much | Replace the Bypass policy with the owner-only Allow. Effective immediately; no deployment. |
| Native login misbehaves after step 4 | Allow policy back first, then `ACCESS_MODE=cloudflare` and restart. Order matters: restore Access before removing the application's own gate, never the other way round. |
| Step 2 or 3 misbehaves | `ACCESS_MODE=cloudflare`, restart. Cloudflare was never changed, so there is nothing to undo there. |
| Step 0 misbehaves | Follow the database recovery procedure in the owner's separately approved production rollout procedure; the pre-0005 binary requires a verified pre-upgrade restore. |

Image rollback does not reverse migrations or change Cloudflare policy. Journal recovery restores its recorded image, environment and deployment files; a separate manual rollback keeps the current authentication settings. The local preflight reproduced the older binary refusing to start after 0005 with “migration 5 was previously applied but is missing in the resolved migrations”. Preserve the migrated database and restore the verified pre-release backup into a new database for that recovery; never edit SQLx history to conceal the incompatibility. Keep Access active throughout this release; changing its policy requires separate owner authorization.

`scripts/deploy-vm.sh` fails a bad release. After mutation, exit 1 means previous image/configuration recovery passed complete verification; exit 2 means unresolved state or unproved recovery. A database restore remains a separate owner-authorized procedure.

## Acceptance, when the cutover is approved

The gate is one test, in a fresh private window:

1. `https://opportunities.alpblkba.dev/login` renders this project's login page, with no Cloudflare one-time-code screen.
2. `https://opportunities.alpblkba.dev/api/stats` answers 401 JSON, not a login page and not data.
3. A wrong password says the same thing a wrong address does.
4. The right password reaches the workspace, and the stats load.
5. Reloading the page stays signed in; `https://opportunities.alpblkba.dev/matches` deep-links in.
6. Sign out, then the back button, reaches the login page and not the workspace.
7. From another device with no session, `/api/applications` answers 401.

Until every one of those passes, production keeps `ACCESS_MODE=cloudflare`.

## Known risks

- **Native login has never run in production.** Cloudflare Access itself is confirmed there — a real one-time-code login reaches authenticated API responses — but every result in this document comes from a local machine and a local PostgreSQL.
- **Rate limiting is per address and in memory.** It bounds guessing at one account; it does not survive a restart, and behind a tunnel there is no client address this process can trust.
- **One account means one password.** There is no second factor in front of the native login. Cloudflare Access provided one; after the cutover, the password is the whole of it. A hardware key or a TOTP step is the obvious next piece of work.
- **Sessions do not pin to a device.** A stolen cookie works until it expires or is revoked.
- **Rate limiting is per address and in memory.** Bounded at 512 addresses with eviction and a fifteen-minute ceiling, so guessing is useless and the denial it can cause the owner is capped — but an attacker who can reach the form can still cost the owner fifteen minutes at a time. The Cloudflare rate-limiting rule recommended above is the answer to that.

## Cutover assessment (2026-10-27)

Production runs `1043ab9` with `ACCESS_MODE=cloudflare` (read from the VM), so every visitor to
`opportunities.alpblkba.dev` meets the Access login of team `damp-math-e853` before the application.
Nothing in this assessment changed Cloudflare or production.

Verified locally on the working tree, `ACCESS_MODE=native` against a throwaway database:
- `/` serves the original landing page and asks only `/api/auth/session`.
- `/workspace` signed out renders the login form, and private `/api` routes return 401, including the
  new `/api/ops/status`.
- A wrong password is refused, the right one opens the workspace, and logout ends the session.

The native suite covers the rest: Argon2id hashing, identical bodies for unknown and wrong
credentials, the per-address limiter, Origin checks on unsafe methods, `__Host-` cookies over HTTPS,
12-hour expiry and revocation.

**Options for a public landing page, and their trade-offs:**

| Option | Second factor kept | Effort | Notes |
| --- | --- | --- | --- |
| Native cutover (this ladder, then Bypass/Everyone) | No: password only | config + one policy | The landing page is at the hostname the owner asked for. Add the login rate-limit rule first. |
| Native cutover after adding TOTP | Yes (TOTP) | a migration, enrolment CLI, a login step, UI | Not implemented. Recommended before relying on password-only access to the CV and the right to submit applications. |
| Separate public hostname for the landing page only | Yes (Cloudflare OTP) | a static host, plus "Log in" linking to the protected hostname | Keeps today's protection unchanged; the landing page lives at a different address. |
| Path-scoped Bypass on `/` only | n/a | none | Does not work: see "Why path-scoped Bypass is the wrong shape". |

**Lockout recovery stays the one documented above.** Put the Allow policy back first, then set
`ACCESS_MODE=cloudflare`. A lost password is reset on the VM with `opportunity user passwd`.
