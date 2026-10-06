use crate::core::errors::{AppResult, Error};
use crate::core::types::{Entries, Fingerprint, IdempotencyKey, ReverseCmd, TransferCmd};
use crate::db::queries::Query;
use crate::db::rows::{Account, AccountType};
use rust_decimal::Decimal;
use sqlx::{PgPool, Postgres, Transaction};
use std::collections::HashMap;

type UserId = i32;
type TxnId = i32;

pub async fn owned_account(
  ex: impl sqlx::PgExecutor<'_>,
  account_id: i32,
  user_id: UserId,
) -> AppResult<Account> {
  let account = Query::get_account(ex, account_id)
    .await?
    .ok_or(Error::AccountNotFound(account_id))?;

  if account.owner_id != user_id {
    return Err(Error::Forbidden("You do not own this account".into()));
  }

  Ok(account)
}

#[tracing::instrument(
  name = "ledger.transfer",
  skip(pool, cmd),
  fields(
    user_id = %user,
    idempotency_key = %key.as_str(),
    from = cmd.source_account_id,
    to = cmd.destination_account_id
  ),
  err
)]
pub async fn transfer(
  pool: &PgPool,
  user: UserId,
  key: &IdempotencyKey,
  cmd: &TransferCmd,
) -> AppResult<TxnId> {
  let fp = Fingerprint::of("transfer", cmd);
  let mut tx = match begin_idempotent(pool, user, key, &fp).await? {
    Begin::Replay(id) => return Ok(id),
    Begin::Fresh(tx) => tx,
  };

  // Ownership check: caller must own the source account
  owned_account(&mut *tx, cmd.source_account_id, user).await?;

  let entries = Entries::transfer(
    cmd.source_account_id,
    cmd.destination_account_id,
    cmd.amount,
  )?;

  let header = Header {
    user_id: user,
    description: cmd.description.as_str(),
    key,
    fingerprint: &fp,
    reverses: None,
  };

  let id = post(&mut tx, header, entries).await?;
  tx.commit().await?;

  metrics::counter!("ledger_transfers_total").increment(1);
  Ok(id)
}

#[tracing::instrument(
  name = "ledger.reverse",
  skip(pool, cmd),
  fields(
    user_id = %user,
    idempotency_key = %key.as_str(),
    target_txn = cmd.target
  ),
  err
)]
pub async fn reverse(
  pool: &PgPool,
  user: UserId,
  key: &IdempotencyKey,
  cmd: &ReverseCmd,
) -> AppResult<TxnId> {
  let fp = Fingerprint::of("reverse", cmd);
  let mut tx = match begin_idempotent(pool, user, key, &fp).await? {
    Begin::Replay(id) => return Ok(id),
    Begin::Fresh(tx) => tx,
  };

  let target = Query::lock_transaction(&mut *tx, cmd.target)
    .await?
    .ok_or(Error::TransactionNotFound(cmd.target))?;

  let entries = Query::get_entries_with_owner(&mut *tx, cmd.target).await?;

  if !entries.iter().any(|e| e.owner_id == user) {
    return Err(Error::Forbidden(
      "You are not authorized to reverse this transaction".into(),
    ));
  }

  if target.reversed_transaction_id.is_some() {
    return Err(Error::CannotReverseReversal(cmd.target));
  }

  if Query::is_reversed(&mut *tx, cmd.target).await? {
    return Err(Error::AlreadyReversed(cmd.target));
  }

  let desc = format!("Reversal of Txn #{}: {}", cmd.target, cmd.reason.as_str());
  let header = Header {
    user_id: user,
    description: &desc,
    key,
    fingerprint: &fp,
    reverses: Some(cmd.target),
  };

  let inverted_entries = Entries::inverse_of(&entries)?;
  let id = post(&mut tx, header, inverted_entries).await?;
  tx.commit().await?;

  metrics::counter!("ledger_reversals_total").increment(1);
  Ok(id)
}

enum Begin {
  Replay(TxnId),
  Fresh(Transaction<'static, Postgres>),
}

struct Header<'a> {
  pub user_id: UserId,
  pub description: &'a str,
  pub key: &'a IdempotencyKey,
  pub fingerprint: &'a Fingerprint,
  pub reverses: Option<TxnId>,
}

async fn begin_idempotent(
  pool: &PgPool,
  user: UserId,
  key: &IdempotencyKey,
  fp: &Fingerprint,
) -> AppResult<Begin> {
  let mut tx = pool.begin().await?;

  // 1. Advisory lock scoped to (user, key)
  Query::acquire_idempotency_lock(&mut *tx, user, key.as_str()).await?;

  // 2. Replay check
  match Query::get_transaction_by_user_and_key(&mut *tx, user, key.as_str()).await? {
    None => Ok(Begin::Fresh(tx)),
    Some(prev) if fp.matches(&prev.request_hash) => {
      metrics::counter!("ledger_idempotent_replays_total").increment(1);
      tx.commit().await?;
      Ok(Begin::Replay(prev.id))
    }
    Some(_) => Err(Error::IdempotencyPayloadMismatch),
  }
}

async fn post(
  tx: &mut Transaction<'static, Postgres>,
  header: Header<'_>,
  entries: Entries,
) -> AppResult<TxnId> {
  let account_ids = entries.account_ids();
  let accounts = Query::lock_accounts(&mut **tx, &account_ids).await?;
  let balances = Query::get_all_balances(&mut **tx, &account_ids).await?;
  check_accounts_and_overdraft(&entries, &accounts, &balances)?;

  let txn = Query::insert_transaction(
    &mut **tx,
    header.user_id,
    header.description,
    header.key.as_str(),
    header.fingerprint.as_bytes(),
    header.reverses,
  )
  .await?;

  // 5. Batch insert entries
  Query::insert_entries(&mut **tx, txn.id, entries).await?;
  Ok(txn.id)
}

fn check_accounts_and_overdraft(
  entries: &Entries,
  accounts: &HashMap<i32, AccountType>,
  balances: &HashMap<i32, Decimal>,
) -> AppResult<()> {
  for e in entries.as_slice().iter() {
    if !accounts.iter().any(|(id, _)| *id == e.account_id) {
      return Err(Error::AccountNotFound(e.account_id));
    }
  }

  for e in entries.as_slice().iter() {
    if e.amount.is_sign_negative()
      && !accounts
        .get(&e.account_id)
        .unwrap()
        .allows_negative_balance()
    {
      let current = balances
        .get(&e.account_id)
        .copied()
        .unwrap_or(Decimal::ZERO);

      if current + e.amount < Decimal::ZERO {
        metrics::counter!("ledger_insufficient_funds_total").increment(1);
        return Err(Error::InsufficientFunds {
          account_id: e.account_id,
          required: -e.amount,
          available: current,
        });
      }
    }
  }
  Ok(())
}
