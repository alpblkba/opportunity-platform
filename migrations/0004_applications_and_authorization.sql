-- Session 4: private applicant facts, documents, application tracking, routing, authorization and notifications.
-- Everything here is user-owned private data. Document bytes and browser artefacts live under the ignored private tree; only metadata and paths are stored.

-- A fact is a claim about the applicant that the owner has approved. Nothing may be filled into a form from an unapproved fact, which is what prevents the system inventing employment history, dates or qualifications.
create table applicant_facts (
    id uuid primary key default gen_random_uuid(),
    owner_id uuid not null references owners (id) on delete cascade,
    key text not null,
    value text not null,
    category text not null default 'other'
        check (category in ('identity', 'contact', 'education', 'experience', 'skill', 'authorization', 'preference', 'other')),
    -- Sensitive facts are never auto-filled without the owner having approved that exact value: citizenship, visa status, disability, criminal history, clearances.
    sensitive boolean not null default false,
    approved boolean not null default false,
    approved_at timestamptz,
    -- Where the claim came from, so every filled field can be traced back to something the owner confirmed.
    provenance text not null default 'manual',
    source_document_id uuid,
    version integer not null default 1,
    -- Deferred because superseding a version and inserting its replacement happen in one transaction: the old row points at the new one, which does not exist until the same transaction creates it.
    superseded_by uuid references applicant_facts (id) on delete set null deferrable initially deferred,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

create unique index applicant_facts_current_idx on applicant_facts (owner_id, key) where superseded_by is null;
create index applicant_facts_approved_idx on applicant_facts (owner_id, approved, category);

create table documents (
    id uuid primary key default gen_random_uuid(),
    owner_id uuid not null references owners (id) on delete cascade,
    kind text not null check (kind in ('cv', 'cover_letter', 'transcript', 'certificate', 'other')),
    label text not null,
    version integer not null default 1,
    -- Path under the ignored private tree. The bytes never enter the database or Git.
    file_path text not null,
    content_hash text not null,
    byte_size integer not null,
    media_type text not null,
    extracted_text text,
    approved boolean not null default false,
    created_at timestamptz not null default now(),
    unique (owner_id, kind, label, version)
);

alter table applicant_facts
    add constraint applicant_facts_document_fk
    foreign key (source_document_id) references documents (id) on delete set null;

-- A reusable answer to a question that recurs across application forms.
create table reusable_answers (
    id uuid primary key default gen_random_uuid(),
    owner_id uuid not null references owners (id) on delete cascade,
    question_key text not null,
    question_text text not null,
    answer text not null,
    sensitive boolean not null default false,
    approved boolean not null default false,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    unique (owner_id, question_key)
);

create table applications (
    id uuid primary key default gen_random_uuid(),
    owner_id uuid not null references owners (id) on delete cascade,
    opportunity_id uuid not null references opportunities (id) on delete cascade,
    profile_id uuid references profiles (id) on delete set null,
    status text not null default 'saved' check (status in (
        'saved', 'preparing', 'needs_review', 'ready', 'submitting',
        'submitted', 'submission_unknown', 'interview', 'rejected', 'withdrawn', 'failed'
    )),
    -- How this application should be completed. Decided from the form and the facts, never from the job type.
    route text check (route in ('auto', 'review', 'curated')),
    route_reason text,
    cv_document_id uuid references documents (id) on delete set null,
    dossier_body text,
    dossier_facts jsonb not null default '[]'::jsonb,
    notes text,
    follow_up_at timestamptz,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    -- One application per opportunity per owner. This is the first of three defences against applying twice.
    unique (owner_id, opportunity_id)
);

create index applications_status_idx on applications (owner_id, status, updated_at desc);

create table application_events (
    id uuid primary key default gen_random_uuid(),
    application_id uuid not null references applications (id) on delete cascade,
    kind text not null,
    detail jsonb not null default '{}'::jsonb,
    created_at timestamptz not null default now()
);

create index application_events_app_idx on application_events (application_id, created_at desc);

-- One try at submitting. The idempotency key is what makes a retried request reuse the existing attempt instead of creating a second submission.
create table application_attempts (
    id uuid primary key default gen_random_uuid(),
    application_id uuid not null references applications (id) on delete cascade,
    idempotency_key text not null unique,
    state text not null default 'started' check (state in ('started', 'filled', 'submitted', 'unknown', 'failed')),
    destination text not null,
    payload_hash text not null,
    authorization_kind text check (authorization_kind in ('policy', 'token')),
    policy_id uuid,
    token_id uuid,
    receipt text,
    evidence jsonb not null default '{}'::jsonb,
    diagnostic text,
    started_at timestamptz not null default now(),
    finished_at timestamptz
);

create index application_attempts_app_idx on application_attempts (application_id, started_at desc);

-- A standing authorization. Off until the owner enables it, revocable at any moment, and narrow by construction: it names the sources it covers and the conditions that must hold.
create table authorization_policies (
    id uuid primary key default gen_random_uuid(),
    owner_id uuid not null references owners (id) on delete cascade,
    name text not null,
    enabled boolean not null default false,
    -- Source keys this policy covers. Empty means it covers nothing, so a policy cannot accidentally be global.
    source_keys text[] not null default '{}',
    -- Every required form field must map to an approved fact before this policy will authorize anything.
    require_all_fields_mapped boolean not null default true,
    -- A form carrying a sensitive question is never auto-submitted under a policy.
    forbid_sensitive_questions boolean not null default true,
    -- A form carrying a substantive free-text question goes to a human instead.
    forbid_free_text boolean not null default true,
    max_per_day integer not null default 5 check (max_per_day between 1 and 100),
    max_total integer check (max_total > 0),
    submissions_made integer not null default 0,
    expires_at timestamptz,
    revoked_at timestamptz,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now(),
    unique (owner_id, name)
);

-- An individual authorization: one use, bound to exactly this application, destination and payload.
create table authorization_tokens (
    id uuid primary key default gen_random_uuid(),
    owner_id uuid not null references owners (id) on delete cascade,
    application_id uuid not null references applications (id) on delete cascade,
    -- Binding the payload hash is what stops a token granted for one set of answers being used for another.
    payload_hash text not null,
    destination text not null,
    source_key text not null,
    expires_at timestamptz not null,
    used_at timestamptz,
    used_by_attempt uuid,
    created_at timestamptz not null default now()
);

create index authorization_tokens_app_idx on authorization_tokens (application_id, used_at);

-- A question the form asked that could not be answered from approved facts. The owner answers it here and the application resumes.
create table review_tasks (
    id uuid primary key default gen_random_uuid(),
    application_id uuid not null references applications (id) on delete cascade,
    field_name text not null,
    question_text text not null,
    field_type text not null default 'text',
    required boolean not null default true,
    sensitive boolean not null default false,
    -- A suggestion assembled from approved facts. Never a guess at a fact the owner has not confirmed.
    draft_answer text,
    answer text,
    status text not null default 'open' check (status in ('open', 'answered', 'dismissed')),
    created_at timestamptz not null default now(),
    resolved_at timestamptz,
    unique (application_id, field_name)
);

create index review_tasks_open_idx on review_tasks (status, created_at);

-- Durable record of something the owner should know about. Delivery to email or a phone is later work; emitting and storing the event is this session's job.
create table notifications (
    id uuid primary key default gen_random_uuid(),
    owner_id uuid not null references owners (id) on delete cascade,
    application_id uuid references applications (id) on delete cascade,
    kind text not null check (kind in ('action_required', 'submitted', 'failed', 'submission_uncertain')),
    -- One notification per application per kind per transition. The unique index is what makes "emitted exactly once" a database guarantee rather than a hope.
    dedupe_key text not null,
    payload jsonb not null default '{}'::jsonb,
    created_at timestamptz not null default now(),
    delivered_at timestamptz,
    unique (owner_id, dedupe_key)
);

create index notifications_undelivered_idx on notifications (owner_id, delivered_at, created_at desc);
