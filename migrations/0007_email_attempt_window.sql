alter table notifications add column first_attempt_at timestamptz;
update notifications set first_attempt_at=last_attempt_at where last_attempt_at is not null;
