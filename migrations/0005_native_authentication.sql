-- Native authentication: accounts, passwords and server-side sessions.
--
-- Until this, identity came entirely from Cloudflare Access and the application had no notion of an
-- account. This adds one, without disturbing the `owners` row that every personal record already
-- points at: a user belongs to an owner, and the owner is what the data belongs to. Existing rows
-- keep their owner and need no migration.
--
-- Nothing here creates an account. There is no default user and no default password: the first
-- account is made by the owner with `opportunity user create`, on the machine.

create table users (
    id uuid primary key default gen_random_uuid(),
    -- What this account's data belongs to. Several accounts may one day share an owner (a person
    -- and their agent); none may read another owner's rows.
    owner_id uuid not null references owners (id) on delete cascade,
    email text not null,
    display_name text not null,
    -- owner: the person this service exists for. user: an additional human account. agent: a
    -- non-interactive client. Only `owner` is permitted until cross-account isolation is enforced
    -- in every query; see docs/AUTHENTICATION.md.
    role text not null default 'owner' check (role in ('owner', 'user', 'agent')),
    -- An Argon2id PHC string. Null means this account cannot log in with a password, which is how
    -- an agent account exists without one.
    password_hash text,
    disabled_at timestamptz,
    created_at timestamptz not null default now(),
    updated_at timestamptz not null default now()
);

-- Addresses are compared case-insensitively, so uniqueness has to be too.
create unique index users_email_key on users (lower(email));

-- At most one owner account exists, enforced by the database rather than by remembering.
--
-- Isolation between accounts is not implemented yet: no query filters by owner_id. Until it is,
-- a second account would read the first one's CV and applications, so the schema refuses to have
-- one. Lifting this is a deliberate change that comes with the query work, not a convenience.
create unique index users_single_owner on users ((role = 'owner')) where role = 'owner';

create table user_sessions (
    id uuid primary key default gen_random_uuid(),
    user_id uuid not null references users (id) on delete cascade,
    -- The SHA-256 of the opaque token, never the token. Someone who reads this table cannot mint a
    -- cookie from it, which is the whole reason the session is opaque rather than a signed claim.
    token_hash bytea not null unique,
    created_at timestamptz not null default now(),
    last_seen_at timestamptz not null default now(),
    expires_at timestamptz not null,
    revoked_at timestamptz
);

create index user_sessions_user_idx on user_sessions (user_id);
-- Expiry sweeps and the lookup on every request both read this.
create index user_sessions_live_idx on user_sessions (expires_at) where revoked_at is null;
