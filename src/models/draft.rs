use crate::{LedgerError, LedgerResult};
use rust_decimal::Decimal;
use rust_decimal::prelude::Zero;
use std::collections::HashSet;

#[derive(Debug, PartialEq)]
pub struct PostingDraft {
  pub account_id: i32,
  pub amount: Decimal,
}

impl PostingDraft {
  pub fn new(account_id: i32, amount: Decimal) -> LedgerResult<Self> {
    if amount.is_zero() {
      return Err(LedgerError::ZeroAmountPosting);
    }
    Ok(Self { account_id, amount })
  }
}

#[derive(Debug, PartialEq)]
pub struct TransactionDraft {
  pub description: String,
  pub idempotency_key: String,
  pub postings: Vec<PostingDraft>,
}

impl TransactionDraft {
  pub fn new(
    description: impl Into<String>,
    idempotency_key: impl Into<String>,
    postings: Vec<PostingDraft>,
  ) -> LedgerResult<Self> {
    let len = postings.len() as i32;
    if len < 2 {
      return Err(LedgerError::InsufficientPostings(len));
    }

    let mut account_ids = HashSet::new();
    let mut total: Decimal = Decimal::zero();

    for posting in postings.iter() {
      if !account_ids.insert(posting.account_id) {
        return Err(LedgerError::DuplicateAccountPosting);
      }
      total += posting.amount;
    }

    if !total.is_zero() {
      return Err(LedgerError::UnbalancedTransaction(total));
    }

    Ok(Self {
      description: description.into(),
      idempotency_key: idempotency_key.into(),
      postings,
    })
  }
}
