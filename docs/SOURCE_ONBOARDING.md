# Adding a source

A source is one employer's job board, identified by whatever token that board provider uses. The identifier is never guessed from a company name: find it on the employer's own public careers page, register it, and let the validation request confirm it answers. A registration that fails validation stays visible as a configuration error rather than becoming a source that silently reports zero jobs.

## Finding the identifier

**Greenhouse.** Open the employer's careers page and follow a job link. A Greenhouse-hosted board has a URL of the form `https://job-boards.greenhouse.io/<board_token>/jobs/<id>` or `https://boards.greenhouse.io/<board_token>`. The `<board_token>` segment is what you register. Confirm it with `curl -s -o /dev/null -w '%{http_code}' https://boards-api.greenhouse.io/v1/boards/<board_token>/jobs`, which answers `200` for a real board and `404` otherwise.

**Lever.** A Lever-hosted job link looks like `https://jobs.lever.co/<site_id>/<uuid>`. Register the `<site_id>` segment. Confirm it with `curl -s -o /dev/null -w '%{http_code}' 'https://api.lever.co/v0/postings/<site_id>?mode=json'`.

**Ashby.** An Ashby-hosted job link looks like `https://jobs.ashbyhq.com/<board_slug>/<uuid>`. Register the `<board_slug>` segment. Confirm it with `curl -s -o /dev/null -w '%{http_code}' https://api.ashbyhq.com/posting-api/job-board/<board_slug>`.

**Workday.** A Workday career site URL looks like `https://<tenant>.<instance>.myworkdayjobs.com/<site>`, for example `https://acme.wd1.myworkdayjobs.com/External`. All three segments are registered separately. The adapter is implemented and tested against five pagination-hazard fixtures, but it is listed as `fixture only` and refuses a live read by default: the interface it reads requires a POST, which falls outside this project's approval for public listing GET requests. To permit a first supervised live read, set `WORKDAY_LIVE_PROBE=1` in the environment before starting the app, watch what it does, and check the tenant's own terms first. Until then a Workday source reports zero and says plainly that zero means untested rather than empty.

**Academic pages.** A university or institute listing has no API, so an academic source carries CSS selectors that say where each row and field is. Use a built-in preset when the page matches one, or supply your own selectors when it does not; no code change is needed either way. The only preset today is `successfactors`, which matches the SAP SuccessFactors career sites many research institutes run. Their rows look like `tr.data-row` containing `a.jobTitle-link`, `span.jobLocation`, `span.jobDepartment` and `span.jobFacility`. To find out whether a page matches, open it and look for those class names in the page source.

For any other layout, register the source with explicit selectors. `row` and `title` are required; `link`, `location`, `department`, `employment_type`, `supervisor` and `body` are optional and absent fields simply stay unknown. A German institute thesis page, for example, might use:

```toml
[[sources]]
key = "example-institut-arbeiten"
kind = "academic_html"
organization = "Example Institut"
url = "https://example-institut.test/abschlussarbeiten"
enabled = true
refresh_hours = 24

[sources.selectors]
row = "article.thesis"
title = "h3 a"
location = ".ort"
department = ".institut"
employment_type = ".art"
supervisor = ".betreuer"
body = ".beschreibung"
```

The adapter reads the listing page and nothing else. It never follows a row through to a job detail or application page, so rows that are only a heading are stored and flagged as having no description rather than being enriched by further requests.

What a row means is read from its own words. A title or body mentioning `Masterarbeit`, `Abschlussarbeit`, `Bachelorarbeit`, `master thesis` or `open topic` is recorded as an open topic, not as a funded vacancy. Wording such as `Forschungsinteressen` or `prospective supervisor` is recorded as a possible supervisor with nothing advertised. A post that states a pay scale such as `TV-L` or `Entgeltgruppe 13` is a real vacancy. A careers site registered with the `successfactors` preset treats its rows as advertised vacancies by default, because that is what a careers site publishes, but a row whose own text says `Masterarbeit` still reads as a topic. A named supervisor is taken only from a labelled field such as `Betreuer:`, `Supervisor:` or `Ansprechpartner:in:`, and a deadline only from a labelled one such as `Bewerbungsfrist:` or `Application deadline:`. Anything unlabelled stays unknown rather than being guessed from a nearby name or date.

**Anything else.** Register the careers page as a `manual` watch entry. A manual entry has no adapter: it records a link for you to follow by hand and it never claims to have discovered jobs. This is how a coverage gap stays visible instead of looking like an employer with no openings.

## Registering

From the UI, open the Sources tab, fill in the kind, a key, the organization name and the identifier, then press `Register and validate`. The form registers the source and immediately makes one read-only request, reporting the outcome and the number of records the board returned.

From the command line:

```bash
./target/release/opportunity source add --kind greenhouse --key acme-greenhouse --organization "Acme" --identifier acme --enabled
./target/release/opportunity source validate acme-greenhouse
./target/release/opportunity scan --source acme-greenhouse
```

To register several at once, put them in a TOML file shaped like `config/sources.example.toml` and run `opportunity source import <path>`. Import never overwrites a source that already exists, and it rejects any entry whose identifier is still a `REPLACE_WITH_...` placeholder.

## What the labels mean

`Verified live` means an adapter actually read this source and the sweep covered the whole board. `Fixture only` means the adapter passes its golden fixtures but this particular source has never been read successfully. `Blocked` means the source refused access; records discovered earlier are kept and are not treated as closed. `Manual link` means there is no adapter at all. `Never checked` means registered and never attempted, so a count of zero says nothing about whether the employer is hiring.

A `Partial sweep` is a read that returned some records but is known to be incomplete, for example because the board reported more jobs than the response carried. A partial sweep stores every record it did see and is never allowed to close a posting, because a record it did not see is unknown rather than gone.

## Rate and access etiquette

Every request is an unauthenticated GET against the provider's documented listing API, identified by a descriptive user agent. There is no authenticated scraping, no bot-protection circumvention and no employer application form is ever opened by the scanner. If a board starts answering `403` or `429`, the source is marked `blocked` or `degraded` with the status in its diagnostic; raise its `refresh_hours` rather than retrying harder. Repeated failures put a source into exponential backoff with jitter, capped at a day, so a failing source is retried less and less often but never abandoned. A single success clears the backoff.

Outbound requests are checked twice. The URL must be plain `https` with no embedded credentials, and the hostname is resolved through a filter that refuses any name whose addresses include a loopback, private, carrier-grade NAT, link-local, unique-local or cloud metadata address. Because the check happens inside DNS resolution, it also covers every redirect hop and leaves no window between validating a name and connecting to it. This matters most for academic sources, where the URL comes from you rather than from a fixed provider template.
