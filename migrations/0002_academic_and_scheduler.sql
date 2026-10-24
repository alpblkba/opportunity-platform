-- Session 2: academic record fields and durable scheduler state.

-- Academic listings publish things an ATS board does not. Each column is nullable because a page that does not state a supervisor or a deadline has not said there is none.
alter table postings
    add column supervisor text,
    add column contact text,
    add column deadline_at timestamptz,
    -- What the source is offering, as the source described it. An ATS posting is an advertised vacancy; a thesis page is usually an open topic, and a supervisor page is neither.
    add column advertised_state text not null default 'advertised'
        check (advertised_state in ('advertised', 'open_topic', 'potential_supervisor', 'unknown'));

alter table opportunities
    add column supervisor text,
    add column contact text;

create index postings_deadline_idx on postings (deadline_at) where deadline_at is not null;
create index opportunities_deadline_idx on opportunities (deadline_at) where deadline_at is not null;

-- Scheduler state lives in the database so a restart resumes exactly where it left off rather than re-scanning everything or forgetting a backoff.
alter table sources
    add column next_run_at timestamptz,
    add column consecutive_failures integer not null default 0,
    -- Set while a source is in backoff after repeated failures; the scheduler skips it until this passes.
    add column backoff_until timestamptz,
    add column last_http_status integer;

-- Existing enabled sources become due immediately so the scheduler picks them up on first start.
update sources set next_run_at = now() where enabled;

create index sources_due_idx on sources (enabled, next_run_at) where enabled;
