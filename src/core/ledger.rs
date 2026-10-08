use crate::core::errors::{AppResult, Error};
use crate::core::types::{Entries, Fingerprint, IdempotencyKey, ReverseCmd, TransferCmd};
use crate::db;
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
  let account = db::get_account(ex, account_id)
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

  let target = db::lock_transaction(&mut *tx, cmd.target)
    .await?
    .ok_or(Error::TransactionNotFound(cmd.target))?;

  let entries = db::get_entries_with_owner(&mut *tx, cmd.target).await?;

  if !entries.iter().any(|e| e.owner_id == user) {
    return Err(Error::Forbidden(
      "You are not authorized to reverse this transaction".into(),
    ));
  }

  if target.reversed_transaction_id.is_some() {
    return Err(Error::CannotReverseReversal(cmd.target));
  }

  if db::is_reversed(&mut *tx, cmd.target).await? {
    return Err(Error::AlreadyReversed);
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
  db::acquire_idempotency_lock(&mut *tx, user, key.as_str()).await?;

  // 2. Replay check
  match db::get_transaction_by_user_and_key(&mut *tx, user, key.as_str()).await? {
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
  let accounts = db::lock_accounts(&mut **tx, &account_ids).await?;
  let balances = db::get_all_balances(&mut **tx, &account_ids).await?;
  check_accounts_and_overdraft(&entries, &accounts, &balances)?;

  let txn = db::insert_transaction(
    &mut **tx,
    header.user_id,
    header.description,
    header.key.as_str(),
    header.fingerprint.as_bytes(),
    header.reverses,
  )
  .await?;

  // 5. Batch insert entries
  db::insert_entries(&mut **tx, txn.id, entries).await?;
  Ok(txn.id)
}

pub(super) fn check_accounts_and_overdraft(
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::core::types::{Amount};
  use crate::db::rows::EntryWithOwner;
  use rust_decimal::dec;

  fn sample_entries(from: i32, to: i32, amount: Amount) -> Entries {
    Entries::transfer(from, to, amount).expect("valid transfer entries")
  }

  #[test]
  fn test_check_accounts_and_overdraft_missing_account() {
    let amt = Amount::try_from(dec!(50.00)).unwrap();
    let entries = sample_entries(1, 2, amt);

    // Only account 1 is provided; account 2 is missing
    let mut accounts = HashMap::new();
    accounts.insert(1, AccountType::Asset);

    let balances = HashMap::from([(1, dec!(100.00))]);

    let res = check_accounts_and_overdraft(&entries, &accounts, &balances);
    assert!(matches!(res, Err(Error::AccountNotFound(2))));
  }

  #[test]
  fn test_check_accounts_and_overdraft_exact_balance_passes() {
    let amt = Amount::try_from(dec!(100.00)).unwrap();
    let entries = sample_entries(1, 2, amt);

    let mut accounts = HashMap::new();
    accounts.insert(1, AccountType::Asset);
    accounts.insert(2, AccountType::Asset);

    // Account 1 has exactly 100.00 available
    let balances = HashMap::from([(1, dec!(100.00)), (2, dec!(0.00))]);

    assert!(check_accounts_and_overdraft(&entries, &accounts, &balances).is_ok());
  }

  #[test]
  fn test_check_accounts_and_overdraft_one_cent_short_fails() {
    let amt = Amount::try_from(dec!(100.00)).unwrap();
    let entries = sample_entries(1, 2, amt);

    let mut accounts = HashMap::new();
    accounts.insert(1, AccountType::Asset);
    accounts.insert(2, AccountType::Asset);

    // Account 1 only has 99.99
    let balances = HashMap::from([(1, dec!(99.99)), (2, dec!(0.00))]);

    let res = check_accounts_and_overdraft(&entries, &accounts, &balances);
    match res {
      Err(Error::InsufficientFunds {
            account_id,
            required,
            available,
          }) => {
        assert_eq!(account_id, 1);
        assert_eq!(required, dec!(100.00));
        assert_eq!(available, dec!(99.99));
      }
      other => panic!("expected InsufficientFunds error, got {other:?}"),
    }
  }

  #[test]
  fn test_check_accounts_and_overdraft_defaults_missing_balance_to_zero() {
    let amt = Amount::try_from(dec!(10.00)).unwrap();
    let entries = sample_entries(1, 2, amt);

    let mut accounts = HashMap::new();
    accounts.insert(1, AccountType::Asset);
    accounts.insert(2, AccountType::Asset);

    // No balance record exists in map for account 1
    let empty_balances = HashMap::new();

    let res = check_accounts_and_overdraft(&entries, &accounts, &empty_balances);
    match res {
      Err(Error::InsufficientFunds {
            account_id,
            required,
            available,
          }) => {
        assert_eq!(account_id, 1);
        assert_eq!(required, dec!(10.00));
        assert_eq!(available, dec!(0.00));
      }
      other => panic!("expected InsufficientFunds error, got {other:?}"),
    }
  }

  #[test]
  fn test_check_accounts_and_overdraft_allows_negative_on_exempt_accounts() {
    let amt = Amount::try_from(dec!(500.00)).unwrap();
    let entries = sample_entries(1, 2, amt);

    // Account 1 is Equity, which allows negative balances
    let mut accounts = HashMap::new();
    accounts.insert(1, AccountType::Equity);
    accounts.insert(2, AccountType::Asset);

    let balances = HashMap::from([(1, dec!(0.00)), (2, dec!(0.00))]);

    assert!(check_accounts_and_overdraft(&entries, &accounts, &balances).is_ok());
  }

  #[test]
  fn test_reversal_entry_inversion_maintains_balance() {
    let original = vec![
      EntryWithOwner {
        account_id: 10,
        owner_id: 1,
        amount: dec!(-45.50),
      },
      EntryWithOwner {
        account_id: 20,
        owner_id: 2,
        amount: dec!(45.50),
      },
    ];

    let inverted = Entries::inverse_of(&original).expect("inversion succeeds");
    let slice = inverted.as_slice();

    assert_eq!(slice.len(), 2);
    assert_eq!(slice[0].account_id, 10);
    assert_eq!(slice[0].amount, dec!(45.50));
    assert_eq!(slice[1].account_id, 20);
    assert_eq!(slice[1].amount, dec!(-45.50));
  }
}