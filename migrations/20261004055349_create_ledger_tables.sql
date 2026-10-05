-- 1. account categories
create type account_type as enum (
    'asset',
    'liability',
    'equity',
    'revenue',
    'expense'
);

-- 2. accounts
create table accounts (
    id serial primary key,
    name varchar(100) not null unique,
    account_type account_type not null,
    created_at timestamptz not null default clock_timestamp()
);

-- 3. transactions (the envelope)
create table transactions (
    id serial primary key,
    description varchar(255) not null,
    created_at timestamptz not null default clock_timestamp()
);

-- 4. entries (the legs)
create table entries (
    id serial primary key,
    transaction_id integer not null references transactions(id) on delete restrict,
    account_id integer not null references accounts(id) on delete restrict,
    amount numeric(12, 2) not null,
    created_at timestamptz not null default clock_timestamp(),
    constraint chk_amount_nonzero check (amount <> 0)
);

create index idx_entries_account_id on entries(account_id);
create index idx_entries_transaction_id on entries(transaction_id);