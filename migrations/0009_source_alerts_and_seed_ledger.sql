-- A source that keeps failing after the scheduler's own retries reaches the owner, with reminders while it lasts.
alter table notifications drop constraint notifications_kind_check;
alter table notifications add constraint notifications_kind_check check (kind in ('action_required', 'submitted', 'failed', 'submission_uncertain', 'manual_required', 'source_failing'));

-- Every seed source a release has ever created, so the startup import adds a source a new release
-- introduces but never recreates one the owner deleted. Sources present at upgrade count as seeded.
create table seed_ledger (
    key text primary key,
    seeded_at timestamptz not null default now()
);
insert into seed_ledger (key) select key from sources;
