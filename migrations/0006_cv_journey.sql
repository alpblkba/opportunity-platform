alter table documents add column candidate jsonb;
create table product_settings (
    owner_id uuid primary key references owners(id),
    profile_id uuid references profiles(id),
    updated_at timestamptz not null default now()
);
alter table authorization_policies add column profile_id uuid references profiles(id);
alter table applications drop constraint applications_status_check;
alter table applications add constraint applications_status_check check (status in (
    'saved','preparing','needs_review','ready','submitting','submitted','submission_unknown',
    'interview','rejected','withdrawn','failed','unsupported'
));
alter table review_tasks add column options jsonb not null default '[]';
alter table notifications add column delivery_state text not null default 'pending';
alter table notifications add column delivery_receipt text;
alter table notifications add column delivery_error text;
alter table notifications add column last_attempt_at timestamptz;
alter table applications add column auto_resume boolean not null default false;
