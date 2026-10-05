use crate::LedgerResult;
use crate::db::schemas::*;
use rust_decimal::Decimal;
use sqlx::{Executor, Postgres};

pub async fn create_account<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  name: &str,
  atype: AccountType,
) -> LedgerResult<Account> {
  let account = sqlx::query_as!(
    Account,
    r#"
		insert into accounts (name, account_type)
		values ($1, $2)
		returning id, name, created_at, account_type
		as "account_type: AccountType";
	"#,
    name,
    atype as AccountType
  )
  .fetch_one(executor)
  .await?;

  Ok(account)
}

pub async fn get_account<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  id: i32,
) -> LedgerResult<Option<Account>> {
  let account = sqlx::query_as!(
    Account,
    r#"
		select id, name, created_at, account_type
		as "account_type:AccountType"
		from accounts where id = $1
	"#,
    id
  )
  .fetch_optional(executor)
  .await?;

  Ok(account)
}

pub async fn lock_accounts<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  account_ids: &[i32],
) -> LedgerResult<Vec<LockedAccountRecord>> {
  let rows = sqlx::query_as!(
    LockedAccountRecord,
    r#"
		select id, account_type
		as "account_type: AccountType"
		from accounts where id = any($1::int[])
		order by id for update
	"#,
    &account_ids
  )
  .fetch_all(executor)
  .await?;

  Ok(rows)
}

pub async fn get_balance<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  account_id: i32,
) -> LedgerResult<Decimal> {
  let balance = sqlx::query_scalar!(
    r#"
		select coalesce(sum(amount), 0.0) as "sum!"
		from entries
		where account_id = $1
		"#,
    account_id
  )
  .fetch_one(executor)
  .await?;

  Ok(balance)
}

pub async fn insert_entries_batch<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  transaction_id: i32,
  account_ids: &[i32],
  amounts: &[Decimal],
) -> LedgerResult<()> {
  let txn_ids = vec![transaction_id; account_ids.len()];

  sqlx::query!(
    r#"
		insert into entries (transaction_id, account_id, amount)
		select * from unnest($1::int[], $2::int[], $3::numeric[])
		"#,
    &txn_ids,
    account_ids,
    amounts as &[Decimal]
  )
  .execute(executor)
  .await?;

  Ok(())
}

pub async fn get_transaction_by_idempotency_key<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  idempotency_key: &str,
) -> LedgerResult<Option<Transaction>> {
  let txn = sqlx::query_as!(
    Transaction,
    r#"
		select
			id,
			description,
			idempotency_key,
			reversed_transaction_id,
			created_at
		from transactions
		where idempotency_key = $1
		"#,
    idempotency_key
  )
  .fetch_optional(executor)
  .await?;

  Ok(txn)
}

pub async fn get_transaction_by_id<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  id: i32,
) -> LedgerResult<Option<Transaction>> {
  let txn = sqlx::query_as!(
    Transaction,
    r#"
		select
			id,
			description,
			idempotency_key,
			reversed_transaction_id,
			created_at
		from transactions
		where id = $1
		"#,
    id
  )
  .fetch_optional(executor)
  .await?;

  Ok(txn)
}

pub async fn insert_transaction<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  description: &str,
  idempotency_key: &str,
) -> LedgerResult<Option<Transaction>> {
  let txn = sqlx::query_as!(
    Transaction,
    r#"
		insert into transactions (description, idempotency_key)
		values ($1, $2)
		on conflict (idempotency_key) do nothing
		returning
			id,
			description,
			idempotency_key,
			reversed_transaction_id,
			created_at
		"#,
    description,
    idempotency_key
  )
  .fetch_optional(executor)
  .await?;

  Ok(txn)
}

pub async fn get_reversal_by_original_id<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  original_id: i32,
) -> LedgerResult<Option<i32>> {
  let rev_id = sqlx::query_scalar!(
    r#"
		select id
		from transactions
		where reversed_transaction_id = $1
		"#,
    original_id
  )
  .fetch_optional(executor)
  .await?;

  Ok(rev_id)
}

pub async fn get_entries_by_transaction_id<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  transaction_id: i32,
) -> LedgerResult<Vec<Entry>> {
  let entries = sqlx::query_as!(
    Entry,
    r#"
		select id, transaction_id, account_id, amount
		from entries
		where transaction_id = $1
		order by id
		"#,
    transaction_id
  )
  .fetch_all(executor)
  .await?;

  Ok(entries)
}

pub async fn insert_reversal_transaction<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  description: &str,
  idempotency_key: &str,
  reversed_transaction_id: i32,
) -> LedgerResult<Option<Transaction>> {
  let txn = sqlx::query_as!(
    Transaction,
    r#"
		insert into transactions (description, idempotency_key, reversed_transaction_id)
		values ($1, $2, $3)
		on conflict (idempotency_key) do nothing
		returning 
		    id, 
		    description, 
		    idempotency_key, 
		    reversed_transaction_id, 
		    created_at
		"#,
    description,
    idempotency_key,
    reversed_transaction_id
  )
  .fetch_optional(executor)
  .await?;

  Ok(txn)
}
