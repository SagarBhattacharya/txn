use super::draft::*;
use crate::LedgerError;
use rust_decimal::{Decimal, dec};

const TEST_IDEMPOTENCY_KEY: &str = "test-key";

#[test]
fn test_correct_posting_draft() {
  let posting = PostingDraft::new(1, dec!(500.00));
  assert!(posting.is_ok());
}

#[test]
fn test_incorrect_posting_draft() {
  let posting = PostingDraft::new(1, Decimal::ZERO);
  assert!(posting.is_err());
}

#[test]
fn less_than_two_postings_are_rejected() {
  let posting = PostingDraft::new(2, dec!(100.0)).unwrap();
  let txn = TransactionDraft::new("invalid transaction", TEST_IDEMPOTENCY_KEY, vec![posting]);

  assert_eq!(txn, Err(LedgerError::InsufficientPostings(1)));
}

#[test]
fn unbalanced_transaction_is_rejected() {
  let credit = PostingDraft::new(1, dec!(100.0)).unwrap();
  let debit = PostingDraft::new(2, dec!(50.0)).unwrap();

  let txn = TransactionDraft::new(
    "unbalanced transaction",
    TEST_IDEMPOTENCY_KEY,
    vec![credit, debit],
  );

  assert_eq!(txn, Err(LedgerError::UnbalancedTransaction(dec!(150.0))));
}

#[test]
fn duplicate_posting_is_rejected() {
  let credit = PostingDraft::new(1, dec!(100.0)).unwrap();
  let debit = PostingDraft::new(1, dec!(-100.0)).unwrap();

  let txn = TransactionDraft::new(
    "duplicate postings in transaction",
    TEST_IDEMPOTENCY_KEY,
    vec![credit, debit],
  );

  assert_eq!(txn, Err(LedgerError::DuplicateAccountPosting));
}

#[test]
fn valid_transaction_successful() {
  let credit = PostingDraft::new(1, dec!(100.0)).unwrap();
  let debit = PostingDraft::new(2, dec!(-100.0)).unwrap();

  let txn = TransactionDraft::new(
    "valid transaction",
    TEST_IDEMPOTENCY_KEY,
    vec![credit, debit],
  );

  assert!(txn.is_ok());
}
