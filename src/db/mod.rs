use crate::config::Config;
use crate::db::recorder::TransactionRecorder;
use crate::db::schemas::*;
use crate::models::draft::{PostingDraft, TransactionDraft};
use crate::models::payload_fg::compute_reversal_hash;
use crate::{LedgerError, LedgerResult};
use rust_decimal::Decimal;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

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

  pub async fn is_user_party_to_transaction(
    &self,
    transaction_id: i32,
    user_id: i32,
  ) -> LedgerResult<bool> {
    queries::is_user_party_to_transaction(&self.pool, transaction_id, user_id).await
  }

  pub async fn record_transaction(&self, draft: TransactionDraft) -> LedgerResult<i32> {
    let mut tx = self.pool.begin().await?;

    // 1. Advisory lock scoped to (user_id, key)
    queries::acquire_idempotency_lock(&mut *tx, draft.user_id, &draft.idempotency_key).await?;

    // 2. Check if key was already used by this user
    if let Some(existing) = queries::get_transaction_by_user_and_key(
      &mut *tx, draft.user_id, &draft.idempotency_key
    ).await? {
      if existing.request_hash != draft.request_hash {
        return Err(LedgerError::IdempotencyPayloadMismatch);
      }
      tx.commit().await?;
      return Ok(existing.id);
    }

    // 3. Record entries
    let mut recorder = TransactionRecorder::new(tx);
    let txn_id = recorder.record(draft, None).await?;
    recorder.commit().await?;

    Ok(txn_id)
  }

  pub async fn reverse_transaction(
    &self,
    user_id: i32,
    target_txn_id: i32,
    reason: &str,
    idempotency_key: impl Into<String>,
  ) -> LedgerResult<i32> {
    let key = idempotency_key.into();
    let request_hash = compute_reversal_hash(target_txn_id, reason);
    let mut tx = self.pool.begin().await?;

    // 1. Advisory lock scoped to (user_id, key)
    queries::acquire_idempotency_lock(&mut *tx, user_id, &key).await?;

    // 2. Replay check
    if let Some(existing) = queries::get_transaction_by_user_and_key(
      &mut *tx, user_id, &key
    ).await? {
      if existing.request_hash != request_hash {
        return Err(LedgerError::IdempotencyPayloadMismatch);
      }
      tx.commit().await?;
      return Ok(existing.id);
    }

    // 3. Target verification & already reversed check...
    let target_txn = queries::get_transaction_by_id(&mut *tx, target_txn_id)
      .await?
      .ok_or(LedgerError::TransactionNotFound(target_txn_id))?;

    if target_txn.reversed_transaction_id.is_some() {
      return Err(LedgerError::CannotReverseReversal(target_txn_id));
    }

    if queries::get_reversal_by_original_id(&mut *tx, target_txn_id).await?.is_some() {
      return Err(LedgerError::AlreadyReversed(target_txn_id));
    }

    let original_entries = queries::get_entries_by_transaction_id(&mut *tx, target_txn_id).await?;
    let inverted_postings = original_entries
      .into_iter()
      .map(|e| PostingDraft::new(e.account_id, -e.amount))
      .collect::<LedgerResult<Vec<_>>>()?;

    let desc = format!("Reversal of Txn #{target_txn_id}: {reason}");
    let draft = TransactionDraft::new(user_id, desc, key, request_hash, inverted_postings)?;

    let mut recorder = TransactionRecorder::new(tx);
    let txn_id = recorder.record(draft, Some(target_txn_id)).await?;
    recorder.commit().await?;

    Ok(txn_id)
  }
}
