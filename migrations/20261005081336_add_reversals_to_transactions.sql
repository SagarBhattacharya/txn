-- Add migration script here
alter table transactions
add column reversed_transaction_id integer unique references transactions(id) on delete restrict;

create index idx_transactions_reversed_id on transactions(reversed_transaction_id);