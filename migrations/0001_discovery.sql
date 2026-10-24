-- Session 1 schema: the discovery half of the product. Profiles, matches, applications and browser-worker tables arrive in their own sessions and migrations.

create extension if not exists "pgcrypto";

-- Single-user locally, but every user-owned row carries an owner from the start so a later multi-user evolution is a data migration rather than a redesign. Public source data deliberately has no owner.
create table owners (
    id uuid primary key default gen_random_uuid(),
    handle text not null unique,
    created_at timestamptz not null default now()
);

insert into owners (handle) values ('local');

create table organizations (
    id uuid primary key default gen_random_uuid(),
    normalized_name text not null unique,
    display_name text not null,
    org_type text not null default 'company' check (org_type in ('company', 'university', 'lab', 'unknown')),
    aliases text[] not null default '{}',
    home_url text,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create table sources (
    id uuid primary key default gen_random_uuid(),
    key text not null unique,
    kind text not null check (kind in ('greenhouse', 'lever', 'ashby', 'workday', 'academic_html', 'manual')),
    display_name text not null,
    organization_id uuid references organizations (id) on delete set null,
    -- Typed endpoint identifiers, serialized from the SourceEndpoint enum. Never a free-form URL for the adapter kinds.
    endpoint jsonb not null,
    enabled boolean not null default false,
    refresh_hours integer not null default 6 check (refresh_hours between 1 and 720),
    tags text[] not null default '{}',
    -- Derived from run history, never asserted by configuration.
    health text not null default 'unknown' check (health in ('unknown', 'healthy', 'degraded', 'blocked', 'schema_error')),
    support text not null default 'not_configured' check (support in ('verified_live', 'fixture_only', 'blocked', 'manual_link', 'not_configured')),
    last_attempt_at timestamptz,
    last_success_at timestamptz,
    last_outcome text,
    last_diagnostic text,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create index sources_enabled_idx on sources (enabled, kind);

create table source_runs (
    id uuid primary key default gen_random_uuid(),
    source_id uuid not null references sources (id) on delete cascade,
    started_at timestamptz not null default now(),
    finished_at timestamptz,
    outcome text not null check (outcome in ('ok_complete', 'ok_verified_empty', 'partial', 'not_modified', 'access_denied', 'schema_error', 'http_error', 'rate_limited')),
    -- Redundant with outcome by design: the disappearance query joins on this one column, so the rule has exactly one place to go wrong.
    complete boolean not null,
    pages_fetched integer not null default 0,
    records_seen integer not null default 0,
    total_reported integer,
    observed_native_ids text[] not null default '{}',
    raw_document_id uuid,
    diagnostic text,
    created_at timestamptz not null default now()
);

create index source_runs_source_time_idx on source_runs (source_id, started_at desc);

create table raw_documents (
    id uuid primary key default gen_random_uuid(),
    source_id uuid not null references sources (id) on delete cascade,
    content_hash text not null,
    retrieved_at timestamptz not null default now(),
    source_url text not null,
    media_type text not null default 'application/json',
    byte_size integer not null,
    -- Payload bodies live under the ignored private/ tree; only the metadata and the path are in the database.
    storage_path text,
    parser_version integer not null default 1,
    unique (source_id, content_hash)
);

alter table source_runs add constraint source_runs_raw_document_fk foreign key (raw_document_id) references raw_documents (id) on delete set null;

create table opportunities (
    id uuid primary key default gen_random_uuid(),
    organization_id uuid not null references organizations (id) on delete cascade,
    dedupe_key text not null,
    canonical_title text not null,
    kind text not null check (kind in ('employment', 'part_time', 'internship', 'working_student', 'master_thesis', 'research_assistant', 'phd', 'fellowship', 'other')),
    advertised_state text not null default 'unknown' check (advertised_state in ('advertised', 'open_topic', 'potential_supervisor', 'unknown')),
    country_code text,
    city text,
    -- The source's own wording about remote work, kept verbatim. Interpreting it into an eligibility decision happens at query time, never at ingest.
    remote_raw text,
    remote_eligibility text not null default 'unknown' check (remote_eligibility in ('on_site', 'hybrid', 'remote_restricted', 'remote_anywhere', 'unknown')),
    deadline_at timestamptz,
    -- When this tool first saw the record, which is not the date the employer published it.
    first_seen_at timestamptz not null default now(),
    last_seen_at timestamptz not null default now(),
    closed_at timestamptz,
    search_text text not null default '',
    -- The 'simple' configuration is deliberate: German and English postings share one index and no stemmer distorts the other language's terms.
    search_vector tsvector generated always as (to_tsvector('simple', search_text)) stored,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    unique (organization_id, dedupe_key)
);

create index opportunities_search_idx on opportunities using gin (search_vector);
create index opportunities_open_idx on opportunities (closed_at, last_seen_at desc);
create index opportunities_facets_idx on opportunities (kind, country_code, city);

create table postings (
    id uuid primary key default gen_random_uuid(),
    source_id uuid not null references sources (id) on delete cascade,
    native_id text not null,
    -- Nullable on purpose: unlinking a posting from its opportunity is how a wrong duplicate merge is reversed without destroying either record.
    opportunity_id uuid references opportunities (id) on delete set null,
    url text not null,
    title text not null,
    description_html text,
    description_text text,
    -- A source row with no usable description stays valid; ranking reads this flag so a low lexical score is understood as missing text, not a poor match.
    title_only boolean not null default false,
    location_raw text,
    country_code text,
    city text,
    remote_raw text,
    department text,
    employment_type_raw text,
    posted_at timestamptz,
    content_hash text not null,
    first_seen_at timestamptz not null default now(),
    last_seen_at timestamptz not null default now(),
    -- Last time a sweep that saw the source's whole inventory included this posting.
    last_complete_seen_at timestamptz,
    -- Consecutive complete sweeps that did not include this posting. Only a complete sweep may change it.
    missing_streak integer not null default 0,
    closed_at timestamptz,
    unique (source_id, native_id)
);

create index postings_opportunity_idx on postings (opportunity_id);
create index postings_source_seen_idx on postings (source_id, last_seen_at desc);

create table opportunity_revisions (
    id uuid primary key default gen_random_uuid(),
    opportunity_id uuid not null references opportunities (id) on delete cascade,
    posting_id uuid not null references postings (id) on delete cascade,
    content_hash text not null,
    change_reason text not null,
    changed_at timestamptz not null default now(),
    diff jsonb not null default '{}'::jsonb
);

create index opportunity_revisions_opportunity_idx on opportunity_revisions (opportunity_id, changed_at desc);
