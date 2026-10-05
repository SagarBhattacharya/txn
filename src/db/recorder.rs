// src/database/recorder.rs
use crate::db::queries;
use crate::db::schemas::AccountType;
use crate::models::draft::{PostingDraft, TransactionDraft};
use crate::{LedgerError, LedgerResult};
use rust_decimal::Decimal;
use sqlx::{Postgres, Transaction};
use std::collections::{HashMap, HashSet};

pub struct TransactionRecorder<'c> {
  tx: Transaction<'c, Postgres>,
  account_types: HashMap<i32, AccountType>,
}

impl<'c> TransactionRecorder<'c> {
  pub fn new(tx: Transaction<'c, Postgres>) -> Self {
    Self {
      tx,
      account_types: HashMap::new(),
    }
  }

  pub async fn record(&mut self, draft: TransactionDraft) -> LedgerResult<i32> {
    // 1. Short-circuit if idempotency key exists
    if let Some(existing) =
      queries::get_transaction_by_idempotency_key(&mut *self.tx, &draft.idempotency_key).await?
    {
      return Ok(existing.id);
    }

    // 2. Lock accounts in deterministic order
    let account_ids: Vec<i32> = draft.postings.iter().map(|p| p.account_id).collect();
    self.lock_and_verify_accounts(&account_ids).await?;

    // 3. Check overdraft thresholds on debited asset accounts
    self.verify_posting_thresholds(&draft.postings).await?;

    // 4. Insert transaction parent header
    let maybe_txn =
      queries::insert_transaction(&mut *self.tx, &draft.description, &draft.idempotency_key)
        .await?;

    let txn_id = match maybe_txn {
      Some(txn) => {
        let amounts: Vec<Decimal> = draft.postings.iter().map(|p| p.amount).collect();
        queries::insert_entries_batch(&mut *self.tx, txn.id, &account_ids, &amounts).await?;
        txn.id
      }
      None => {
        let existing =
          queries::get_transaction_by_idempotency_key(&mut *self.tx, &draft.idempotency_key)
            .await?
            .expect("Transaction must exist on conflict");
        existing.id
      }
    };

    Ok(txn_id)
  }

  pub async fn record_reversal(
    &mut self,
    draft: TransactionDraft,
    target_txn_id: i32,
  ) -> LedgerResult<i32> {
    // 1. Short-circuit if idempotency key exists
    if let Some(existing) =
      queries::get_transaction_by_idempotency_key(&mut *self.tx, &draft.idempotency_key).await?
    {
      return Ok(existing.id);
    }

    // 2. Lock accounts involved in the reversal (ascending order)
    let account_ids: Vec<i32> = draft.postings.iter().map(|p| p.account_id).collect();
    self.lock_and_verify_accounts(&account_ids).await?;

    // 3. Verify liquidity/overdraft rules: the party losing funds must actually have enough balance!
    self.verify_posting_thresholds(&draft.postings).await?;

    // 4. Insert transaction with reversed_transaction_id linked
    let maybe_txn = queries::insert_reversal_transaction(
      &mut *self.tx,
      &draft.description,
      &draft.idempotency_key,
      target_txn_id,
    )
    .await?;

    let txn_id = match maybe_txn {
      Some(txn) => {
        let amounts: Vec<Decimal> = draft.postings.iter().map(|p| p.amount).collect();
        queries::insert_entries_batch(&mut *self.tx, txn.id, &account_ids, &amounts).await?;
        txn.id
      }
      None => {
        let existing =
          queries::get_transaction_by_idempotency_key(&mut *self.tx, &draft.idempotency_key)
            .await?
            .expect("Transaction must exist on conflict");
        existing.id
      }
    };

    Ok(txn_id)
  }

  pub async fn commit(self) -> LedgerResult<()> {
    self.tx.commit().await?;
    Ok(())
  }

  async fn lock_and_verify_accounts(&mut self, account_ids: &[i32]) -> LedgerResult<()> {
    let mut sorted_ids = account_ids.to_vec();
    sorted_ids.sort_unstable();
    sorted_ids.dedup();

    let rows = queries::lock_accounts(&mut *self.tx, &sorted_ids).await?;

    if rows.len() != sorted_ids.len() {
      let found_ids: HashSet<i32> = rows.iter().map(|r| r.id).collect();
      for expected_id in sorted_ids {
        if !found_ids.contains(&expected_id) {
          return Err(LedgerError::AccountNotFound(expected_id));
        }
      }
    }

    self.account_types = rows.into_iter().map(|r| (r.id, r.account_type)).collect();
    Ok(())
  }

  async fn verify_posting_thresholds(&mut self, postings: &[PostingDraft]) -> LedgerResult<()> {
    for posting in postings {
      let acc_type = self
        .account_types
        .get(&posting.account_id)
        .ok_or(LedgerError::AccountNotFound(posting.account_id))?;

      if *acc_type == AccountType::Asset && posting.amount.is_sign_negative() {
        let current_balance = queries::get_balance(&mut *self.tx, posting.account_id).await?;

        if current_balance + posting.amount < Decimal::ZERO {
          return Err(LedgerError::InsufficientFunds {
            account_id: posting.account_id,
            current_balance,
            attempted_delta: posting.amount,
          });
        }
      }
    }

    Ok(())
  }
}
