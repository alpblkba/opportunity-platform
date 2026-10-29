-- The owner's marks on opportunities. Kept apart from ranking: saving or flagging one for review
-- changes nothing about what is ranked. Hiding one stays an exclusion rule ('hidden').
create table opportunity_marks (
    owner_id uuid not null references owners (id) on delete cascade,
    opportunity_id uuid not null references opportunities (id) on delete cascade,
    mark text not null check (mark in ('saved', 'review')),
    created_at timestamptz not null default now(),
    primary key (owner_id, opportunity_id, mark)
);

-- One row per owner, created with these defaults on first use: a daily digest of new matches at or
-- above priority 60 at 08:00 in Europe/Berlin, and deadline reminders 7, 3 and 1 day ahead.
create table notification_preferences (
    owner_id uuid primary key references owners (id) on delete cascade,
    new_match_digest boolean not null default true,
    min_priority integer not null default 60 check (min_priority between 0 and 100),
    digest_hour integer not null default 8 check (digest_hour between 0 and 23),
    time_zone text not null default 'Europe/Berlin',
    deadline_reminders boolean not null default true,
    reminder_days integer[] not null default '{7,3,1}',
    -- When the digest was last evaluated, sent or not; matches first seen after it are new.
    last_digest_at timestamptz,
    updated_at timestamptz not null default now()
);

-- What "new since your last visit" is measured against.
alter table product_settings add column last_visit_at timestamptz;

alter table notifications drop constraint notifications_kind_check;
alter table notifications add constraint notifications_kind_check check (kind in ('action_required', 'submitted', 'failed', 'submission_uncertain', 'manual_required', 'source_failing', 'new_matches', 'deadline_reminder'));
