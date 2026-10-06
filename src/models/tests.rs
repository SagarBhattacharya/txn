use super::draft::*;
use crate::LedgerError;
use rust_decimal::{dec, Decimal};
use crate::models::payload_fg::{compute_reversal_hash, compute_transfer_hash};

const TEST_USER_ID: i32 = 1;
const TEST_IDEMPOTENCY_KEY: &str = "test-key";

fn dummy_hash() -> Vec<u8> {
  vec![0u8; 32]
}

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
  let txn = TransactionDraft::new(
    TEST_USER_ID,
    "invalid transaction",
    TEST_IDEMPOTENCY_KEY,
    dummy_hash(),
    vec![posting],
  );

  assert_eq!(txn, Err(LedgerError::InsufficientPostings(1)));
}

#[test]
fn unbalanced_transaction_is_rejected() {
  let credit = PostingDraft::new(1, dec!(100.0)).unwrap();
  let debit = PostingDraft::new(2, dec!(50.0)).unwrap();

  let txn = TransactionDraft::new(
    TEST_USER_ID,
    "unbalanced transaction",
    TEST_IDEMPOTENCY_KEY,
    dummy_hash(),
    vec![credit, debit],
  );

  assert_eq!(txn, Err(LedgerError::UnbalancedTransaction(dec!(150.0))));
}

#[test]
fn duplicate_posting_is_rejected() {
  let credit = PostingDraft::new(1, dec!(100.0)).unwrap();
  let debit = PostingDraft::new(1, dec!(-100.0)).unwrap();

  let txn = TransactionDraft::new(
    TEST_USER_ID,
    "duplicate postings in transaction",
    TEST_IDEMPOTENCY_KEY,
    dummy_hash(),
    vec![credit, debit],
  );

  assert_eq!(txn, Err(LedgerError::DuplicateAccountPosting));
}

#[test]
fn valid_transaction_successful() {
  let credit = PostingDraft::new(1, dec!(100.0)).unwrap();
  let debit = PostingDraft::new(2, dec!(-100.0)).unwrap();

  let txn = TransactionDraft::new(
    TEST_USER_ID,
    "valid transaction",
    TEST_IDEMPOTENCY_KEY,
    dummy_hash(),
    vec![credit, debit],
  );

  assert!(txn.is_ok());
}

#[test]
fn test_compute_transfer_hash_determinism_and_sensitivity() {
  let h1 = compute_transfer_hash(1, 2, dec!(100.00), "Payment");
  let h2 = compute_transfer_hash(1, 2, dec!(100.00), "Payment");
  let h3_diff_amount = compute_transfer_hash(1, 2, dec!(100.01), "Payment");
  let h4_diff_account = compute_transfer_hash(1, 3, dec!(100.00), "Payment");
  let h5_trimmed_desc = compute_transfer_hash(1, 2, dec!(100.00), "  Payment  ");

  assert_eq!(h1, h2, "Identical transfer inputs must yield identical hashes");
  assert_eq!(h1, h5_trimmed_desc, "Trimming must produce identical hashes");
  assert_ne!(h1, h3_diff_amount, "Amount change must alter hash");
  assert_ne!(h1, h4_diff_account, "Account change must alter hash");
  assert_eq!(h1.len(), 32, "SHA-256 output must be 32 bytes");
}

#[test]
fn test_compute_reversal_hash_determinism_and_sensitivity() {
  let r1 = compute_reversal_hash(42, "Customer cancellation");
  let r2 = compute_reversal_hash(42, "Customer cancellation");
  let r3_diff_id = compute_reversal_hash(43, "Customer cancellation");
  let r4_diff_reason = compute_reversal_hash(42, "Wrong item received");
  let r5_trimmed = compute_reversal_hash(42, "   Customer cancellation   ");

  assert_eq!(r1, r2, "Identical reversal inputs must yield identical hashes");
  assert_eq!(r1, r5_trimmed, "Trimming reason must produce identical hashes");
  assert_ne!(r1, r3_diff_id, "Target transaction ID change must alter hash");
  assert_ne!(r1, r4_diff_reason, "Reason change must alter hash");
  assert_eq!(r1.len(), 32, "SHA-256 output must be 32 bytes");
}