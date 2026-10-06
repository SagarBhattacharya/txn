use crate::LedgerResult;
use crate::db::schemas::*;
use rust_decimal::Decimal;
use sqlx::{Executor, Postgres};

pub async fn create_user<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  username: &str,
  password_hash: &str,
) -> LedgerResult<Option<User>> {
  let user = sqlx::query_as!(
    User,
    r#"
    insert into users (username, password_hash)
    values ($1, $2)
    on conflict (username) do nothing
    returning id, username, password_hash, created_at
    "#,
    username,
    password_hash
  )
    .fetch_optional(executor)
    .await?;

  Ok(user)
}

pub async fn get_user_by_username<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  username: &str,
) -> LedgerResult<Option<User>> {
  let user = sqlx::query_as!(
    User,
    r#"
    select id, username, password_hash, created_at
    from users
    where username = $1
    "#,
    username
  )
    .fetch_optional(executor)
    .await?;

  Ok(user)
}

pub async fn get_user_by_id<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  id: i32,
) -> LedgerResult<Option<User>> {
  let user = sqlx::query_as!(
    User,
    r#"
    select id, username, password_hash, created_at
    from users
    where id = $1
    "#,
    id
  )
    .fetch_optional(executor)
    .await?;

  Ok(user)
}

pub async fn create_account<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  owner_id: i32,
  name: &str,
  atype: AccountType,
) -> LedgerResult<Option<Account>> {
  let account = sqlx::query_as!(
    Account,
    r#"
		insert into accounts (owner_id, name, account_type)
		values ($1, $2, $3)
		on conflict (owner_id, name) do nothing
		returning id, owner_id, name, created_at, account_type
		as "account_type: AccountType";
	"#,
    owner_id,
    name,
    atype as AccountType
  )
  .fetch_optional(executor)
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
		select id, owner_id, name, created_at, account_type
		as "account_type:AccountType"
		from accounts where id = $1
	"#,
    id
  )
  .fetch_optional(executor)
  .await?;

  Ok(account)
}

pub async fn get_accounts_by_owner<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  owner_id: i32,
) -> LedgerResult<Vec<Account>> {
  let accounts = sqlx::query_as!(
    Account,
    r#"
    select id, owner_id, name, created_at, account_type
    as "account_type: AccountType" from accounts
    where owner_id = $1 order by id
    "#,
    owner_id
  )
    .fetch_all(executor)
    .await?;

  Ok(accounts)
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

pub async fn get_transaction_by_user_and_key<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  user_id: i32,
  key: &str,
) -> LedgerResult<Option<Transaction>> {
  let txn = sqlx::query_as!(
    Transaction,
    r#"
    select
        id,
        user_id,
        description,
        idempotency_key,
        request_hash,
        reversed_transaction_id,
        created_at
    from transactions
    where user_id = $1 AND idempotency_key = $2
    "#,
    user_id,
    key
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
			user_id,
			description,
			idempotency_key,
			request_hash,
			reversed_transaction_id,
			created_at
		from transactions
		where id = $1 for update
		"#,
    id
  )
  .fetch_optional(executor)
  .await?;

  Ok(txn)
}

pub async fn insert_transaction<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  user_id: i32,
  description: &str,
  idempotency_key: &str,
  request_hash: &[u8],
  reversed_transaction_id: Option<i32>,
) -> LedgerResult<Option<Transaction>> {
  let txn = sqlx::query_as!(
    Transaction,
    r#"
    insert into transactions (user_id, description, idempotency_key, request_hash, reversed_transaction_id)
    values ($1, $2, $3, $4, $5)
    on conflict (user_id, idempotency_key) do nothing
    returning id, user_id, description, idempotency_key, request_hash, reversed_transaction_id, created_at
    "#,
    user_id,
    description,
    idempotency_key,
    request_hash,
    reversed_transaction_id
  )
    .fetch_optional(executor)
    .await?;

  Ok(txn)
}

pub async fn acquire_idempotency_lock<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  user_id: i32,
  key: &str,
) -> LedgerResult<()> {
  // Postgres 2-argument advisory lock: (int4, int4)
  sqlx::query!(
    "select pg_advisory_xact_lock($1, hashtext($2))",
    user_id,
    key
  )
    .execute(executor)
    .await?;

  Ok(())
}

pub async fn is_user_party_to_transaction<'e>(
  executor: impl Executor<'e, Database = Postgres>,
  transaction_id: i32,
  user_id: i32,
) -> LedgerResult<bool> {
  let is_party = sqlx::query_scalar!(
    r#"
    select exists (
      select 1
      from entries e
      join accounts a on a.id = e.account_id
      where e.transaction_id = $1 and a.owner_id = $2
    ) as "exists!"
    "#,
    transaction_id,
    user_id
  )
    .fetch_one(executor)
    .await?;

  Ok(is_party)
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