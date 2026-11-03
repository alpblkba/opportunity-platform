# Source onboarding

A source identifies one employer board or academic listing page. Register provider identifiers from the employer's public careers site rather than deriving them from a company name. Source configuration, adapter output and ingestion are separate contracts; a successful parser test does not establish live reachability or inventory completeness.

## Supported source kinds

| Kind | Configuration | Listing access |
| --- | --- | --- |
| `greenhouse` | `board_token` | GET `https://boards-api.greenhouse.io/v1/boards/<board_token>/jobs?content=true` |
| `lever` | `site_id` | GET `https://api.lever.co/v0/postings/<site_id>?mode=json&limit=<page-size>&skip=<offset>` |
| `ashby` | `board_slug` | GET `https://api.ashbyhq.com/posting-api/job-board/<board_slug>` |
| `workday` | `tenant`, `instance`, `site` | Paginated POST to the tenant's Workday listing endpoint |
| `academic_html` | `url`, plus `pattern` or `selectors` | GET listing pages using CSS selectors |
| `manual` | `url` | No adapter; a link for manual inspection |

Greenhouse board tokens appear in `https://job-boards.greenhouse.io/<board_token>/jobs/<id>` or `https://boards.greenhouse.io/<board_token>`. Lever and Ashby identifiers appear in `https://jobs.lever.co/<site_id>/<uuid>` and `https://jobs.ashbyhq.com/<board_slug>/<uuid>`. For Workday, `https://<tenant>.<instance>.myworkdayjobs.com/<site>` supplies the three fields.

Use the adapter validation path to check responses. An HTTP 200 alone does not establish that the payload parses or that every page was retrieved. A `manual` source does not discover jobs or imply the employer has no vacancies.

### Workday access

[workday.rs](../crates/adapters/src/workday.rs) reads listing results through POST with `appliedFacets`, `limit`, `offset` and `searchText`. Pages contain at most 20 requested records, and a scan stops after at most 25 pages. It checks totals, duplicate IDs, empty middle pages, short pages and changing totals before declaring completeness. It does not open application forms.

The current code does not read `WORKDAY_LIVE_PROBE` or enforce an opt-in through that variable. Registering a Workday source initially sets `support=fixture_only`, but validation or a scan can make live requests immediately, and enabled sources are scheduled. This label is not a network-access control. Obtain authorization for the provider's listing access and review its terms before validating or enabling a source; leave it disabled until that review is complete. The CLI `source add` does not accept the three Workday fields; use the UI or TOML import.

## Registration and validation

Source configuration is defined by [SourceEndpoint](../crates/domain/src/source.rs), [NewSource](../crates/core/src/sources.rs) and the [TOML importer](../crates/core/src/config.rs). Endpoint validation rejects empty fields and values containing `REPLACE_WITH`. Greenhouse, Lever and Ashby identifiers permit only ASCII letters, digits, hyphens and underscores. `refresh_hours` must be from 1 through 720 and defaults to 6. Changing an existing source's endpoint kind is refused; register a new source instead.

In the UI, use the source form under settings to register and validate. Registration persists the source, but validation only returns the adapter's outcome, completeness, record count and diagnostic. It does not ingest records or update stored run history, health or support. Validation invokes a full adapter scan, so it may make multiple listing requests. A failed validation must be investigated before scheduled ingestion is enabled.

The CLI uses the configured `DATABASE_URL` and an initialized database:

```bash
./target/release/opportunity source add --kind greenhouse --key acme-greenhouse --organization "Acme" --identifier acme
./target/release/opportunity source validate acme-greenhouse
./target/release/opportunity scan --source acme-greenhouse
./target/release/opportunity source enable acme-greenhouse
```

These commands use an illustrative identifier; replace it with a verified board token. Explicit `scan --source` ingests data even when the source is disabled. `source validate` does not persist scan results. CLI validation and scanning do not acquire the scheduler work lock, and CLI scans do not update its schedule or use its provider pacing. Avoid running them concurrently with production refresh or browser work. `--enabled` on `source add` schedules the source immediately, so omit it when validation should precede scheduled access.

For multiple sources, follow [config/sources.example.toml](../config/sources.example.toml) and run:

```bash
./target/release/opportunity source import path/to/sources.toml
```

Import skips existing keys and reports rejected entries. A manual import can recreate a deleted source. The compiled release seed, [config/sources.seed.toml](../config/sources.seed.toml), is imported at startup using a ledger that prevents previously seeded and subsequently deleted sources from being recreated. Neither path overwrites an existing source's configuration.

## Academic listing configuration

The only built-in preset is `successfactors`. It selects `tr.data-row`, `a.jobTitle-link`, `span.jobLocation`, `span.jobDepartment`, `span.jobFacility`, `span.paginationLabel` and `ul.pagination a`. Explicit selectors take precedence over a preset's selector set.

`row` and `title` are required. Optional row fields are `link`, `location`, `department`, `employment_type`, `supervisor` and `body`; `total` selects a page result counter and `page_links` selects listing pagination links. Missing fields remain unknown. For example:

```toml
[[sources]]
key = "example-institut-arbeiten"
kind = "academic_html"
organization = "Example Institut"
url = "https://example-institut.test/abschlussarbeiten"
enabled = false
refresh_hours = 24

[sources.selectors]
row = "article.thesis"
title = "h3 a"
location = ".ort"
department = ".institut"
employment_type = ".art"
supervisor = ".betreuer"
body = ".beschreibung"
total = ".result-count"
page_links = "nav.pagination a"
```

Selectors must match the actual page. The adapter follows listing pagination, up to ten pages, but does not fetch individual job details or application pages. Repeated native IDs are deduplicated. A stated total larger than the retrieved count yields `partial`; with no total, an unfinished queue or page-cap stop also yields `partial`. With no remaining detected pagination, the adapter can report completion without a total. Missing or incorrect pagination selectors can therefore overstate completeness and must be tested against a representative fixture.

No matching rows on the first page produce `schema_error`, not verified empty. Failure after earlier pages returned records produces `partial` and retains those records. The stored raw payload is not necessarily the full paginated inventory: the academic adapter retains the first page and Workday retains its latest successful payload. Keep multiple fixtures when testing pagination.

Academic classification uses the row's text. Thesis or open-topic wording such as `Masterarbeit`, `Abschlussarbeit`, `Bachelorarbeit`, `master thesis` and `open topic` produces `open_topic`; research-interest or prospective-supervisor wording can produce `potential_supervisor`. Vacancy wording such as `TV-L` or `Entgeltgruppe 13` can establish `advertised`. The `successfactors` preset defaults otherwise unclassified rows to `advertised`; explicit row wording takes precedence. These classifications do not independently verify funding.

A supervisor comes from a dedicated selector or an explicit label such as `Betreuer:`, `Supervisor:` or `Ansprechpartner:in:`. Deadlines come from recognized labels such as `Bewerbungsfrist:` or `Application deadline:`. Names and dates elsewhere in the page are not sufficient evidence. See [academic parsing](../crates/domain/src/academic.rs).

## Adapter contract

Adapters are functions dispatched by [adapters::scan](../crates/adapters/src/lib.rs), not implementations of a shared adapter trait. Each receives the shared `HttpClient` and its endpoint fields and returns a [ScanResult](../crates/domain/src/types.rs):

- `outcome`: the completeness or failure classification.
- `postings`: normalized `FetchedPosting` records with stable `native_id`, URL, title and optional source-provided fields.
- `stats`: `pages_fetched`, `records_seen` and optional `total_reported`.
- `diagnostic`: a useful failure or incompleteness explanation without credentials.
- `raw_body` and `raw_url`: optional payload provenance for snapshot storage.

Keep HTTP retrieval separate from parsing so synthetic payloads can be replayed offline. Missing descriptions are valid records; fewer than 40 stripped characters sets `title_only`. Do not turn unknown location, remote eligibility, deadline or advertised state into a positive assertion.

For a new provider, add its endpoint and kind handling in the domain, configuration importer and adapter dispatcher. Update registration paths, scheduler selection and pacing, and ingestion's ATS classification where applicable. Add fixtures and integration coverage before claiming live support. Discovery support does not imply browser-form support or submission authorization.

## Ingestion and deduplication

[ingest.rs](../crates/core/src/ingest.rs) persists a scan's run, postings, revisions and disappearance decisions in one transaction. Source attempt metadata is updated after that transaction commits. Duplicate native IDs within a scan are processed once. A posting's stable identity is `(source_id, native_id)`.

Descriptions are stripped to text for matching. Cross-source opportunities use a SHA-256 key over normalized organization, title and stated location, with separators between fields. Unknown location groups only with other unknown-location records; it does not merge with a known city. Organization normalization and title/location normalization are in [normalize.rs](../crates/domain/src/normalize.rs).

Content revisions use a separate hash of title, stripped description, raw location and URL. A metadata-only change outside those fields does not necessarily create a revision. Seen postings reset `missing_streak` and reopen previously closed records.

Only `ok_complete` and `ok_verified_empty` may increment disappearance counts. A posting closes after two complete sweeps miss it. Partial, blocked, failed and `not_modified` results cannot close unseen postings. An opportunity closes only when it has no remaining open postings. These rules are defined by [completeness.rs](../crates/domain/src/completeness.rs) and ingestion; an adapter must not report incomplete inventory as complete.

## Source health and scheduling

| Outcome | Derived health | Inventory implication |
| --- | --- | --- |
| `ok_complete` | `healthy` | Complete nonempty scan; disappearance checks allowed |
| `ok_verified_empty` | `healthy` | Confirmed empty inventory; disappearance checks allowed |
| `not_modified` | `healthy` | No new inventory evidence; no disappearance checks |
| `partial` | `degraded` | Retain observed records; preserve unseen records |
| `http_error`, `rate_limited` | `degraded` | Transport/server failure or HTTP 429 |
| `access_denied` | `blocked` | Access refusal; shared HTTP mapping includes 401, 403 and 404 |
| `schema_error` | `schema_error` | Unexpected response structure or listing layout |

Support labels record a different dimension: `verified_live` follows a persisted complete scan, `fixture_only` is the initial Workday label, `manual_link` means no adapter, and `not_configured` means an adapter source has not yet earned live verification. `blocked` can follow an access refusal. Stored `manual_link` and `fixture_only` labels have special retention rules in `record_attempt`, so consult `last_outcome` and `last_diagnostic` as well as the label. Historical `verified_live` does not prove the latest scan succeeded.

The scheduler checks due enabled sources every 60 seconds and persists `next_run_at`, `consecutive_failures` and `backoff_until`. It scans sources sequentially and uses a shared PostgreSQL work lock. Provider pacing inserts a minimum two-second interval between scheduled source scans for the same provider; it is not a global interval between every pagination request or validation call.

Failure backoff starts at five minutes and doubles, with its base capped at 24 hours. Deterministic source-ID jitter adds up to about 9.8 percent, so the actual delay can exceed 24 hours. Healthy outcomes and partial scans reset the scheduler failure count and use the configured refresh interval; a partial scan remains degraded even though it does not trigger failure backoff.

Enabled sources with at least three consecutive failures produce notifications. Reminder thresholds are 1, 3, 7, 14 and 30 days since the previous success, or source creation if none, followed by 30-day intervals. Notification keys prevent repeated ticks from duplicating the same outage reminder. Increase `refresh_hours` or resolve configuration when a provider refuses or rate-limits access; do not retry aggressively.

## Network restrictions

The shared [HTTP client](../crates/adapters/src/client.rs) identifies itself with the `opportunity-platform/<version>` user agent, uses a 25-second request timeout and 10-second connection timeout, and limits each response body to 8 MiB. Provider entry URLs are checked against an allowed host or host suffix. Redirects are bounded and checked for public HTTPS destinations, but are not restricted to the original provider host.

[net.rs](../crates/adapters/src/net.rs) rejects non-HTTPS URLs, embedded credentials and private-address destinations. Its resolver rejects a hostname if any returned address is prohibited, including loopback, private, carrier-grade NAT, link-local and relevant IPv6 ranges. The guarded resolver also applies on redirect hops. Use this client for new adapters rather than introducing a separate unrestricted request path.

Listing access is unauthenticated. Greenhouse, Lever, Ashby and academic adapters use GET; Workday uses listing-query POST. No discovery adapter bypasses login, CAPTCHA or bot protection, or opens employer application forms.

## Testing and acceptance

Use synthetic fixtures under [crates/adapters/tests/fixtures](../crates/adapters/tests/fixtures/README.md). Include successful parsing, missing optional fields, changed schema, truncated pagination, duplicate IDs, inconsistent totals and failures after an earlier successful page where the provider supports pagination. Verify title-only and academic classification behavior where applicable.

For ingestion changes, extend [discovery.rs](../crates/core/tests/discovery.rs) to verify deduplication, updates, reopening and the rule that incomplete scans cannot close missing postings. Test endpoint validation and network restrictions without credentials or live employer submissions.

```bash
cargo test --locked -p opportunity-domain
cargo test --locked -p opportunity-adapters
# Use an isolated test PostgreSQL database through DATABASE_URL.
cargo test --locked -p opportunity-core --test discovery
```

The [CI workflow](../.github/workflows/hygiene.yml) runs workspace tests against PostgreSQL and supplies a deterministic DNS fixture. Fixture success, a configured CI job and a successful live source read are separate evidence. Record a live source as verified only after a complete scan has been ingested; validation output alone does not update that status.
