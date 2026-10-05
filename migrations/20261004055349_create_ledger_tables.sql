create table if not exists users (
    id serial primary key,
    username varchar(64) unique not null,
    password_hash varchar(255) not null,
    created_at timestamptz not null default now()
);

create type account_type
as enum ('asset', 'liability', 'equity', 'revenue', 'expense');

create table if not exists accounts (
    id serial primary key,
    owner_id integer not null references users(id) on delete restrict,
    name varchar(128) not null,
    account_type account_type not null,
    created_at timestamptz not null default clock_timestamp(),
    constraint uq_accounts_owner_name unique (owner_id, name)
);

create index idx_accounts_owner_id on accounts(owner_id);

create table if not exists transactions (
    id serial primary key,
    description text not null,
    idempotency_key varchar(128) unique,
    reversed_transaction_id integer unique references transactions(id) on delete restrict,
    created_at timestamptz not null default now()
);

create index idx_transactions_reversed_id on transactions(reversed_transaction_id);

create table entries (
    id serial primary key,
    transaction_id integer not null references transactions(id) on delete restrict,
    account_id integer not null references accounts(id) on delete restrict,
    amount numeric(12, 2) not null check (amount <> 0),
    created_at timestamptz not null default clock_timestamp()
);

create index idx_entries_account_id on entries(account_id);
create index idx_entries_transaction_id on entries(transaction_id);