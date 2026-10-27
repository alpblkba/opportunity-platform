-- Owner-wide brakes above every employer policy: a pause that stops all submissions, and a daily cap on automatic ones.
alter table product_settings
    add column submissions_paused boolean not null default false,
    add column max_automatic_per_day integer not null default 10 check (max_automatic_per_day between 0 and 100);
-- Manual completion now reaches the owner as a notification, like failures and uncertain outcomes.
alter table notifications drop constraint notifications_kind_check;
alter table notifications add constraint notifications_kind_check check (kind in ('action_required', 'submitted', 'failed', 'submission_uncertain', 'manual_required'));
