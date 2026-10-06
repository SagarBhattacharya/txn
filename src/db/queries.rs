use crate::core::types::Entries;
use crate::db::rows::*;
use rust_decimal::Decimal;
use sqlx::{PgExecutor, Result};
use std::collections::HashMap;

pub struct Query;

impl Query {
  pub async fn create_user(
    ex: impl PgExecutor<'_>,
    username: &str,
    password_hash: &str,
  ) -> Result<User> {
    sqlx::query_as!(
      User,
      r#"
			insert into users (username, password_hash)
			values ($1, $2)
			returning *
		"#,
      username,
      password_hash
    )
    .fetch_one(ex)
    .await
  }

  pub async fn get_user_by_username(
    ex: impl PgExecutor<'_>,
    username: &str,
  ) -> Result<Option<User>> {
    sqlx::query_as!(
      User,
      r#"
			select * from users
			where username = $1
    "#,
      username
    )
    .fetch_optional(ex)
    .await
  }

  pub async fn create_account(
    ex: impl PgExecutor<'_>,
    owner_id: i32,
    name: &str,
    atype: AccountType,
  ) -> Result<Account> {
    sqlx::query_as!(
      Account,
      r#"
			insert into accounts (owner_id, name, account_type)
			values ($1, $2, $3)
			returning id, owner_id, name, created_at,
			account_type as "account_type: AccountType"
		"#,
      owner_id,
      name,
      atype as AccountType
    )
    .fetch_one(ex)
    .await
  }

  pub async fn get_account(ex: impl PgExecutor<'_>, id: i32) -> Result<Option<Account>> {
    sqlx::query_as!(
      Account,
      r#"
			select id, owner_id, name, created_at, account_type
			as "account_type:AccountType"
			from accounts where id = $1
		"#,
      id
    )
    .fetch_optional(ex)
    .await
  }

  pub async fn get_accounts_by_owner<'e>(
    ex: impl PgExecutor<'_>,
    owner_id: i32,
  ) -> Result<Vec<Account>> {
    sqlx::query_as!(
      Account,
      r#"
			select id, owner_id, name, created_at, account_type
			as "account_type: AccountType" 
			from accounts where owner_id = $1 order by id
		"#,
      owner_id
    )
    .fetch_all(ex)
    .await
  }

  pub async fn get_all_balances(
    ex: impl PgExecutor<'_>,
    account_ids: &[i32],
  ) -> Result<HashMap<i32, Decimal>> {
    let rows = sqlx::query!(
      r#"
			select account_id, coalesce(sum(amount), 0.0) as "balance!"
			from entries
    	where account_id = any($1::int[])
    	group by account_id
    "#,
      account_ids
    )
    .fetch_all(ex)
    .await?;

    Ok(
      rows
        .into_iter()
        .map(|r| (r.account_id, r.balance))
        .collect(),
    )
  }

  pub async fn get_balance(ex: impl PgExecutor<'_>, account_id: i32) -> Result<Decimal> {
    sqlx::query_scalar!(
      r#"
			select coalesce(sum(amount), 0.0) as "sum!"
			from entries
			where account_id = $1
		"#,
      account_id
    )
    .fetch_one(ex)
    .await
  }

  pub async fn lock_accounts(
    ex: impl PgExecutor<'_>,
    account_ids: &[i32],
  ) -> Result<HashMap<i32, AccountType>> {
    let rows = sqlx::query!(
      r#"
			select id,
			account_type as "account_type: AccountType"
			from accounts where id = any($1::int[])
			order by id for update
		"#,
      account_ids
    )
    .fetch_all(ex)
    .await?;

    Ok(rows.into_iter().map(|r| (r.id, r.account_type)).collect())
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

    sqlx::query!(
      r#"
			insert into entries (transaction_id, account_id, amount)
			select * from unnest($1::int[], $2::int[], $3::numeric[])
		"#,
      &txn_ids,
      account_ids.as_slice(),
      amounts.as_slice()
    )
    .execute(ex)
    .await?;

    Ok(())
  }

  pub async fn get_transaction_by_user_and_key(
    ex: impl PgExecutor<'_>,
    user_id: i32,
    key: &str,
  ) -> Result<Option<Transaction>> {
    sqlx::query_as!(
      Transaction,
      r#"
	    select * from transactions
	    where user_id = $1 AND idempotency_key = $2
	  "#,
      user_id,
      key
    )
    .fetch_optional(ex)
    .await
  }

  pub async fn lock_transaction(
    ex: impl PgExecutor<'_>,
    txn_id: i32,
  ) -> Result<Option<Transaction>> {
    sqlx::query_as!(
      Transaction,
      r#"
			select *
			from transactions
			where id = $1 for update
		"#,
      txn_id
    )
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
    sqlx::query_as!(
      Transaction,
      r#"
			insert into transactions
				(user_id, description, idempotency_key,
				 request_hash, reversed_transaction_id)
			values ($1, $2, $3, $4, $5)
			returning *
    "#,
      user_id,
      description,
      idempotency_key,
      request_hash,
      reversed_transaction_id
    )
    .fetch_one(ex)
    .await
  }

  pub async fn acquire_idempotency_lock(
    ex: impl PgExecutor<'_>,
    user_id: i32,
    idempotency_key: &str,
  ) -> Result<()> {
    sqlx::query!(
      "select pg_advisory_xact_lock($1, hashtext($2))",
      user_id,
      idempotency_key
    )
    .execute(ex)
    .await?;
    Ok(())
  }

  pub async fn is_reversed(ex: impl PgExecutor<'_>, txn_id: i32) -> Result<bool> {
    let rev_id = sqlx::query_scalar!(
      r#"
			select id from transactions
			where reversed_transaction_id = $1
		"#,
      txn_id
    )
    .fetch_optional(ex)
    .await?;

    Ok(rev_id.is_some())
  }

  pub async fn get_entries_with_owner(
    ex: impl PgExecutor<'_>,
    txn_id: i32,
  ) -> Result<Vec<EntryWithOwner>> {
    sqlx::query_as!(
      EntryWithOwner,
      r#"
	    select e.account_id, e.amount, a.owner_id
	    from entries e
	    join accounts a on a.id = e.account_id
	    where e.transaction_id = $1
	    order by e.id
	  "#,
      txn_id
    )
    .fetch_all(ex)
    .await
  }
}
