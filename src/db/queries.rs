use crate::core::types::Entries;
use crate::db::rows::*;
use rust_decimal::Decimal;
use sqlx::{PgExecutor, Result};
use std::collections::HashMap;

pub async fn create_user(
  ex: impl PgExecutor<'_>,
  username: &str,
  password_hash: &str,
) -> Result<User> {
  sqlx::query_as(
    r#"
			insert into users (username, password_hash)
			values ($1, $2)
			returning *
		"#,
  )
  .bind(username)
  .bind(password_hash)
  .fetch_one(ex)
  .await
}

pub async fn get_user_by_username(ex: impl PgExecutor<'_>, username: &str) -> Result<Option<User>> {
  sqlx::query_as(
    r#"
			select * from users
			where username = $1
    "#,
  )
  .bind(username)
  .fetch_optional(ex)
  .await
}

pub async fn create_account(
  ex: impl PgExecutor<'_>,
  owner_id: i32,
  name: &str,
  atype: AccountType,
) -> Result<Account> {
  sqlx::query_as(
    r#"
			insert into accounts (owner_id, name, account_type)
			values ($1, $2, $3)
			returning *
		"#,
  )
  .bind(owner_id)
  .bind(name)
  .bind(atype)
  .fetch_one(ex)
  .await
}

pub async fn get_account(ex: impl PgExecutor<'_>, id: i32) -> Result<Option<Account>> {
  sqlx::query_as(
    r#"
			select *
			from accounts where id = $1
		"#,
  )
  .bind(id)
  .fetch_optional(ex)
  .await
}

pub async fn get_accounts_by_owner<'e>(
  ex: impl PgExecutor<'_>,
  owner_id: i32,
) -> Result<Vec<Account>> {
  sqlx::query_as(
    r#"
			select *
			from accounts
			where owner_id = $1
			order by id
		"#,
  )
  .bind(owner_id)
  .fetch_all(ex)
  .await
}

pub async fn get_all_balances(
  ex: impl PgExecutor<'_>,
  account_ids: &[i32],
) -> Result<HashMap<i32, Decimal>> {
  let rows: Vec<(i32, Decimal)> = sqlx::query_as(
    r#"
			select account_id, coalesce(sum(amount), 0.0)
			from entries
    	where account_id = any($1::int[])
    	group by account_id
    "#,
  )
  .bind(account_ids)
  .fetch_all(ex)
  .await?;

  Ok(rows.into_iter().map(|r| (r.0, r.1)).collect())
}

pub async fn get_balance(ex: impl PgExecutor<'_>, account_id: i32) -> Result<Decimal> {
  sqlx::query_scalar(
    r#"
			select coalesce(sum(amount), 0.0)
			from entries
			where account_id = $1
		"#,
  )
  .bind(account_id)
  .fetch_one(ex)
  .await
}

pub async fn lock_accounts(
  ex: impl PgExecutor<'_>,
  account_ids: &[i32],
) -> Result<HashMap<i32, AccountType>> {
  let rows: Vec<(i32, AccountType)> = sqlx::query_as(
    r#"
			select id, account_type
			from accounts where id = any($1::int[])
			order by id for update
		"#,
  )
  .bind(account_ids)
  .fetch_all(ex)
  .await?;

  Ok(rows.into_iter().map(|r| (r.0, r.1)).collect())
}

pub async fn insert_entries(
  ex: impl PgExecutor<'_>,
  transaction_id: i32,
  entries: Entries,
) -> Result<()> {
  let entry_slice = entries.into_inner();
  let txn_ids = vec![transaction_id; entry_slice.len()];
  let account_ids: Vec<i32> = entry_slice.iter().map(|id| id.account_id).collect();
  let amounts: Vec<Decimal> = entry_slice.iter().map(|id| id.amount).collect();

  sqlx::query(
    r#"
			insert into entries (transaction_id, account_id, amount)
			select * from unnest($1::int[], $2::int[], $3::numeric[])
		"#,
  )
  .bind(&txn_ids)
  .bind(&account_ids)
  .bind(&amounts)
  .execute(ex)
  .await?;

  Ok(())
}

pub async fn get_transaction_by_user_and_key(
  ex: impl PgExecutor<'_>,
  user_id: i32,
  key: &str,
) -> Result<Option<Transaction>> {
  sqlx::query_as(
    r#"
	    select * from transactions
	    where user_id = $1 AND idempotency_key = $2
	  "#,
  )
  .bind(user_id)
  .bind(key)
  .fetch_optional(ex)
  .await
}

pub async fn lock_transaction(ex: impl PgExecutor<'_>, txn_id: i32) -> Result<Option<Transaction>> {
  sqlx::query_as(
    r#"
			select *
			from transactions
			where id = $1 for update
		"#,
  )
  .bind(txn_id)
  .fetch_optional(ex)
  .await
}

pub async fn insert_transaction(
  ex: impl PgExecutor<'_>,
  user_id: i32,
  description: &str,
  idempotency_key: &str,
  request_hash: &[u8],
  reversed_transaction_id: Option<i32>,
) -> Result<Transaction> {
  sqlx::query_as(
    r#"
			insert into transactions
				(user_id, description, idempotency_key,
				 request_hash, reversed_transaction_id)
			values ($1, $2, $3, $4, $5)
			returning *
    "#,
  )
  .bind(user_id)
  .bind(description)
  .bind(idempotency_key)
  .bind(request_hash)
  .bind(reversed_transaction_id)
  .fetch_one(ex)
  .await
}

pub async fn acquire_idempotency_lock(
  ex: impl PgExecutor<'_>,
  user_id: i32,
  idempotency_key: &str,
) -> Result<()> {
  sqlx::query("select pg_advisory_xact_lock($1, hashtext($2))")
    .bind(user_id)
    .bind(idempotency_key)
    .execute(ex)
    .await?;
  Ok(())
}

pub async fn is_reversed(ex: impl PgExecutor<'_>, txn_id: i32) -> Result<bool> {
  let rev_id: Option<i32> = sqlx::query_scalar(
    r#"
			select id from transactions
			where reversed_transaction_id = $1
		"#,
  )
  .bind(txn_id)
  .fetch_optional(ex)
  .await?;

  Ok(rev_id.is_some())
}

pub async fn get_entries_with_owner(
  ex: impl PgExecutor<'_>,
  txn_id: i32,
) -> Result<Vec<EntryWithOwner>> {
  sqlx::query_as(
    r#"
	    select e.account_id, e.amount, a.owner_id
	    from entries e
	    join accounts a on a.id = e.account_id
	    where e.transaction_id = $1
	    order by e.id
	  "#,
  )
  .bind(txn_id)
  .fetch_all(ex)
  .await
}

pub async fn get_account_activity(
  ex: impl PgExecutor<'_>,
  account_id: i32,
) -> Result<Vec<AccountActivity>> {
  sqlx::query_as(
    r#"
      select t.id as transaction_id, t.description, e.amount, t.created_at,
             t.reversed_transaction_id as reverses
      from entries e
      join transactions t on t.id = e.transaction_id
      where e.account_id = $1
      order by e.id desc
      limit 50
    "#,
  )
  .bind(account_id)
  .fetch_all(ex)
  .await
}
