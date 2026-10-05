-- Add migration script here
alter table transactions
add column idempotency_key varchar(128) unique;