-- Notifications from before email delivery existed are kept, but are never emailed. Migration 0006
-- added the outbox and gave every existing row delivery_state 'pending', so turning mail on would
-- send that whole backlog at once.
--
-- "Before" is when 0006 was applied, which SQLx records in installed_on at the start of that
-- migration's transaction. Every row the earlier application created precedes it, and every row
-- created since follows it. That boundary is also right on a database that ran 0006 to 0010 long
-- ago: what the upgraded application has queued since then stays deliverable.
--
-- Only rows the outbox never touched change, and only in columns 0006 added:
--   * delivery_state 'pending' becomes 'suppressed';
--   * delivery_error records why.
-- Left alone:
--   * created_at, payload and delivered_at;
--   * a row marked seen (delivered_at is set). Dispatch skips it already.
--   * a row with a send attempt or a provider receipt. The outbox keeps confirming or retiring it
--     under its own idempotency rules.
-- A suppressed row is no longer 'pending', so running this again changes nothing.
update notifications
set delivery_state = 'suppressed',
    delivery_error = 'Created before email delivery existed; kept in the app and not emailed.'
where delivery_state = 'pending'
  and delivered_at is null
  and delivery_receipt is null
  and first_attempt_at is null
  and last_attempt_at is null
  and created_at < (select installed_on from _sqlx_migrations where version = 6);
