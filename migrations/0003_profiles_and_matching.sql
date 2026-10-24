-- Session 3: user-owned search profiles, exclusion rules, feedback, digests and the embedding cache.
-- Everything here belongs to an owner. The single local owner created in migration 0001 is used by default, and the ownership column is what makes a later multi-user evolution a data migration rather than a redesign.

create table profiles (
    id uuid primary key default gen_random_uuid(),
    owner_id uuid not null references owners (id) on delete cascade,
    key text not null,
    display_name text not null,
    priority integer not null default 100,
    enabled boolean not null default true,

    job_kinds text[] not null default '{}',
    terms text[] not null default '{}',
    negative_terms text[] not null default '{}',
    preferred_technologies text[] not null default '{}',
    excluded_technologies text[] not null default '{}',

    preferred_countries text[] not null default '{}',
    preferred_cities text[] not null default '{}',
    -- Where the owner currently is. Used only to decide whether a region-locked remote role is workable, and never published or sent anywhere.
    current_country text,
    relocation_willing boolean not null default false,
    remote_policy text not null default 'any'
        check (remote_policy in ('any', 'remote_from_current_country_only', 'on_site_or_relocate')),
    -- Keep opportunities whose location could not be parsed. On by default: a parsing gap must not hide a real opening.
    include_unknown_location boolean not null default true,

    min_years_experience integer,
    max_years_experience integer,
    -- An advertised experience level is the employer's wish, so by default it only moves the score.
    seniority_policy text not null default 'preference' check (seniority_policy in ('preference', 'hard', 'ignore')),
    education_level text,
    work_authorization text[] not null default '{}',

    weights jsonb not null default '{}'::jsonb,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    unique (owner_id, key)
);

create index profiles_owner_idx on profiles (owner_id, priority, key);

create table exclusion_rules (
    id uuid primary key default gen_random_uuid(),
    owner_id uuid not null references owners (id) on delete cascade,
    -- Null applies the rule to every profile; a profile id scopes it to one.
    profile_id uuid references profiles (id) on delete cascade,
    kind text not null check (kind in ('company', 'title_term', 'description_term', 'hidden')),
    value text not null,
    -- Normalized form used for matching. For a company this is the folded identity, so an exact alias blocks and an unrelated substring does not.
    normalized_value text not null,
    opportunity_id uuid references opportunities (id) on delete cascade,
    note text,
    enabled boolean not null default true,
    created_at timestamptz not null default now()
);

create index exclusion_rules_owner_idx on exclusion_rules (owner_id, enabled, kind);
create unique index exclusion_rules_hidden_idx on exclusion_rules (owner_id, opportunity_id)
    where kind = 'hidden';

create table opportunity_feedback (
    id uuid primary key default gen_random_uuid(),
    owner_id uuid not null references owners (id) on delete cascade,
    profile_id uuid references profiles (id) on delete set null,
    opportunity_id uuid not null references opportunities (id) on delete cascade,
    verdict text not null check (verdict in ('interested', 'not_interested')),
    -- Whether the opportunity was shown by the ranking or drawn from the unranked remainder. Recording this is what makes a later recall measurement honest rather than a survey of what was already surfaced.
    sampled_from text not null default 'surfaced' check (sampled_from in ('surfaced', 'not_surfaced')),
    note text,
    created_at timestamptz not null default now(),
    unique (owner_id, profile_id, opportunity_id)
);

create index opportunity_feedback_profile_idx on opportunity_feedback (profile_id, verdict);

create table digests (
    id uuid primary key default gen_random_uuid(),
    owner_id uuid not null references owners (id) on delete cascade,
    profile_id uuid references profiles (id) on delete cascade,
    generated_at timestamptz not null default now(),
    format text not null check (format in ('markdown', 'csv')),
    item_count integer not null default 0,
    deadline_count integer not null default 0,
    body text not null
);

create index digests_owner_idx on digests (owner_id, generated_at desc);

-- Embeddings are the one expensive artefact in matching, so they are cached by the content they describe. Stored as a plain float array rather than through pgvector because the matcher reranks a bounded shortlist instead of searching the whole corpus; an approximate-nearest-neighbour index only earns its cost in the latter case.
create table opportunity_embeddings (
    opportunity_id uuid not null references opportunities (id) on delete cascade,
    model text not null,
    content_hash text not null,
    vector real[] not null,
    created_at timestamptz not null default now(),
    primary key (opportunity_id, model)
);
