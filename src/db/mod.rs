use crate::config::Config;
use crate::db::recorder::TransactionRecorder;
use crate::db::schemas::*;
use crate::models::draft::{PostingDraft, TransactionDraft};
use crate::{LedgerError, LedgerResult};
use rust_decimal::Decimal;
use sqlx::{Executor, PgPool, Postgres};
use sqlx::postgres::PgPoolOptions;

mod queries;
mod recorder;
pub mod schemas;

#[derive(Debug, Clone)]
pub struct Repo {
  pool: PgPool,
}

impl Repo {
  pub async fn new(config: &Config) -> LedgerResult<Self> {
    let pool = PgPoolOptions::new()
      .max_connections(config.max_db_connections)
      .connect(config.database_url.as_str())
      .await?;

    Ok(Self { pool })
  }

  pub fn from_pool(pool: PgPool) -> Self {
    Self { pool }
  }

  pub fn pool(&self) -> &PgPool {
    &self.pool
  }

  pub async fn create_user(
    &self, 
    username: &str, 
    password_hash: &str
  ) -> LedgerResult<Option<User>> {
    queries::create_user(&self.pool, username, password_hash).await
  }

  pub async fn get_user_by_username(&self, username: &str) -> LedgerResult<Option<User>> {
    queries::get_user_by_username(&self.pool, username).await
  }

  pub async fn get_user_by_id(&self, id: i32) -> LedgerResult<Option<User>> {
    queries::get_user_by_id(&self.pool, id).await
  }

  pub async fn create_account(
    &self,
    owner_id: i32,
    name: &str,
    atype: AccountType,
  ) -> LedgerResult<Option<Account>> {
    queries::create_account(&self.pool, owner_id, name, atype).await
  }

  pub async fn get_account_by_id(&self, id: i32) -> LedgerResult<Option<Account>> {
    queries::get_account(&self.pool, id).await
  }

  pub async fn get_accounts_by_owner(&self, owner_id: i32) -> LedgerResult<Vec<Account>> {
    queries::get_accounts_by_owner(&self.pool, owner_id).await
  }

  pub async fn get_account_balance(&self, id: i32) -> LedgerResult<Decimal> {
    queries::get_balance(&self.pool, id).await
  }

  pub async fn get_transaction_by_id(&self, id: i32) -> LedgerResult<Option<Transaction>> {
    queries::get_transaction_by_id(&self.pool, id).await
  }

  pub async fn get_entries_by_transaction_id(
    &self, 
    transaction_id: i32
  ) -> LedgerResult<Vec<Entry>> {
    queries::get_entries_by_transaction_id(&self.pool, transaction_id).await
  }

  pub async fn is_user_party_to_transaction<'e>(
    &self,
    transaction_id: i32,
    user_id: i32,
  ) -> LedgerResult<bool> {
    queries::is_user_party_to_transaction(&self.pool, transaction_id, user_id).await
  }

  pub async fn record_transaction(&self, draft: TransactionDraft) -> LedgerResult<i32> {
    let tx = self.pool.begin().await?;
    let mut recorder = TransactionRecorder::new(tx);

    let txn_id = recorder.record(draft).await?;
    recorder.commit().await?;

    Ok(txn_id)
  }

  pub async fn reverse_transaction(
    &self,
    target_txn_id: i32,
    reason: &str,
    idempotency_key: impl Into<String>,
  ) -> LedgerResult<i32> {
    let key = idempotency_key.into();
    let mut tx = self.pool.begin().await?;

    // 0. Idempotency replay check FIRST
    if let Some(existing) = queries::get_transaction_by_idempotency_key(&mut *tx, &key).await? {
      tx.commit().await?;
      return Ok(existing.id);
    }

    // 1. Target transaction must exist
    let target_txn = queries::get_transaction_by_id(&mut *tx, target_txn_id)
      .await?
      .ok_or(LedgerError::TransactionNotFound(target_txn_id))?;

    // 2. Target cannot be a reversal itself
    if target_txn.reversed_transaction_id.is_some() {
      return Err(LedgerError::CannotReverseReversal(target_txn_id));
    }

    // 3. Target must not already have been reversed by another transaction
    if queries::get_reversal_by_original_id(&mut *tx, target_txn_id)
      .await?
      .is_some()
    {
      return Err(LedgerError::AlreadyReversed(target_txn_id));
    }

    // 4. Fetch original entries
    let original_entries = queries::get_entries_by_transaction_id(&mut *tx, target_txn_id).await?;

    // 5. Invert legs: delta * -1
    let inverted_postings = original_entries
      .into_iter()
      .map(|e| PostingDraft::new(e.account_id, -e.amount))
      .collect::<LedgerResult<Vec<_>>>()?;

    let desc = format!("Reversal of Txn #{target_txn_id}: {reason}");
    let draft = TransactionDraft::new(desc, key, inverted_postings)?;

    // 6. Record reversal under locks & commit
    let mut recorder = TransactionRecorder::new(tx);
    let rev_id = recorder.record_reversal(draft, target_txn_id).await?;
    recorder.commit().await?;

    Ok(rev_id)
  }
}
