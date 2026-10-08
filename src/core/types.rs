use crate::core::errors::{AppResult, Error};
use crate::db::rows::EntryWithOwner;
use rust_decimal::{Decimal, dec};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

const MAX_AMOUNT: Decimal = dec!(9999999999.99);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferCmd {
  pub source_account_id: i32,
  pub destination_account_id: i32,
  pub amount: Amount,
  pub description: Note,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReverseCmd {
  pub target: i32,
  pub reason: Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint(pub Vec<u8>);

impl Fingerprint {
  pub fn of<T: Serialize>(kind: &str, cmd: &T) -> Self {
    let bytes = serde_json::to_vec(&(kind, cmd)).expect("commands always serialize");

    Self(Sha256::digest(bytes).to_vec())
  }

  pub fn as_bytes(&self) -> &[u8] {
    &self.0
  }
  pub fn matches(&self, stored: &[u8]) -> bool {
    self.0 == stored
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Decimal")]
pub struct Amount(Decimal);

impl Amount {
  pub fn get(&self) -> Decimal {
    self.0
  }
}

impl TryFrom<Decimal> for Amount {
  type Error = Error;

  fn try_from(d: Decimal) -> Result<Self, Self::Error> {
    let d = d.normalize();

    if d <= Decimal::ZERO {
      let msg = "Transfer amount must be strictly greater than zero";
      return Err(Error::BadRequest(msg.into()));
    }
    if d.scale() > 2 {
      let msg = "Transfer amount cannot exceed 2 decimal places";
      return Err(Error::BadRequest(msg.into()));
    }
    if d > MAX_AMOUNT {
      let msg = "Transfer amount exceeds maximum allowed limit (9999999999.99)";
      return Err(Error::BadRequest(msg.into()));
    }
    Ok(Self(d))
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewEntry {
  pub account_id: i32,
  pub amount: Decimal,
}

impl NewEntry {
  pub fn outflow(account_id: i32, a: Amount) -> Self {
    Self {
      account_id,
      amount: -a.get(),
    }
  }

  pub fn inflow(account_id: i32, a: Amount) -> Self {
    Self {
      account_id,
      amount: a.get(),
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entries(Vec<NewEntry>);

impl Entries {
  pub fn new(entries: Vec<NewEntry>) -> AppResult<Self> {
    if entries.len() < 2 {
      return Err(Error::InsufficientPostings(entries.len()));
    }

    let mut seen_accounts = HashSet::with_capacity(entries.len());
    let mut total = Decimal::ZERO;

    for entry in &entries {
      if entry.amount.is_zero() {
        return Err(Error::ZeroAmountPosting);
      }
      if !seen_accounts.insert(entry.account_id) {
        return Err(Error::DuplicateAccountPosting);
      }
      total += entry.amount;
    }

    if !total.is_zero() {
      return Err(Error::UnbalancedTransaction(total));
    }
    Ok(Self(entries))
  }

  pub fn transfer(from: i32, to: i32, a: Amount) -> AppResult<Self> {
    Self::new(vec![NewEntry::outflow(from, a), NewEntry::inflow(to, a)])
  }

  pub fn account_ids(&self) -> Vec<i32> {
    let mut account_ids: Vec<i32> = self.0.iter().map(|e| e.account_id).collect();

    account_ids.sort_unstable();
    account_ids
  }

  pub fn inverse_of(original: &[EntryWithOwner]) -> AppResult<Self> {
    let inverted = original
      .iter()
      .map(|e| NewEntry {
        account_id: e.account_id,
        amount: -e.amount,
      })
      .collect();

    Self::new(inverted)
  }

  pub fn into_inner(self) -> Vec<NewEntry> {
    self.0
  }
  pub fn as_slice(&self) -> &[NewEntry] {
    &self.0
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct Username(String);

impl Username {
  pub fn as_str(&self) -> &str {
    &self.0
  }
}

impl TryFrom<String> for Username {
  type Error = Error;

  fn try_from(raw: String) -> Result<Self, Self::Error> {
    let clean = trimmed(raw, 3, 64, "username")?;
    if !clean.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
      return Err(Error::BadRequest(
        "username can only contain alphanumeric characters and underscores".into(),
      ));
    }
    Ok(Self(clean))
  }
}

#[derive(Debug, Deserialize)]
#[serde(try_from = "String")]
pub struct Password(SecretString);

impl TryFrom<String> for Password {
  type Error = Error;
  fn try_from(s: String) -> Result<Self, Error> {
    let n = s.chars().count();
    if !(8..=128).contains(&n) {
      return Err(Error::BadRequest(
        "password must be between 8 and 128 characters".into(),
      ));
    }
    Ok(Self(SecretString::from(s)))
  }
}

impl Password {
  pub fn expose(&self) -> &str {
    self.0.expose_secret()
  }
}

macro_rules! text_newtype {
  ($name:ident, $what:literal, $min:expr, $max:expr) => {
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(try_from = "String")]
    pub struct $name(String);

    impl TryFrom<String> for $name {
      type Error = Error;
      fn try_from(s: String) -> Result<Self, Error> {
        trimmed(s, $min, $max, $what).map(Self)
      }
    }

    impl std::str::FromStr for $name {
      type Err = Error;
      fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_owned())
      }
    }

    impl $name {
      pub fn as_str(&self) -> &str {
        &self.0
      }
    }
  };
}

text_newtype!(Note, "note", 1, 256);
text_newtype!(AccountName, "account name", 1, 128);
text_newtype!(IdempotencyKey, "idempotency-key", 1, 128);

fn trimmed(raw: String, min: usize, max: usize, field_name: &str) -> AppResult<String> {
  let s = raw.trim();
  let count = s.chars().count();

  if count < min || count > max {
    return Err(Error::BadRequest(format!(
      "{field_name} must be between {min} and {max} characters"
    )));
  }

  Ok(s.to_string())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::str::FromStr;

  #[test]
  fn test_amount_validation() {
    // Valid: positive, scale <= 2, within MAX_AMOUNT limit
    assert!(Amount::try_from(dec!(0.01)).is_ok());
    assert!(Amount::try_from(dec!(100.50)).is_ok());
    assert!(Amount::try_from(dec!(100.000)).is_ok()); // Normalizes to 100
    assert!(Amount::try_from(dec!(9999999999.99)).is_ok());

    // Invalid: zero or negative
    assert!(matches!(
      Amount::try_from(dec!(0.00)),
      Err(Error::BadRequest(_))
    ));
    assert!(matches!(
      Amount::try_from(dec!(-5.00)),
      Err(Error::BadRequest(_))
    ));

    // Invalid: sub-cent precision (> 2 decimal places after normalization)
    assert!(matches!(
      Amount::try_from(dec!(10.005)),
      Err(Error::BadRequest(_))
    ));

    // Invalid: exceeds MAX_AMOUNT
    assert!(matches!(
      Amount::try_from(dec!(10000000000.00)),
      Err(Error::BadRequest(_))
    ));
  }

  #[test]
  fn test_text_newtypes_trimming_and_bounds() {
    // Note bounds: 1..=256
    assert!(Note::from_str("").is_err());
    assert!(Note::from_str("   ").is_err());
    assert_eq!(
      Note::from_str("  Transfer for dinner  ").unwrap().as_str(),
      "Transfer for dinner"
    );
    assert!(Note::from_str(&"a".repeat(256)).is_ok());
    assert!(Note::from_str(&"a".repeat(257)).is_err());

    // AccountName bounds: 1..=128
    assert!(AccountName::from_str("").is_err());
    assert_eq!(
      AccountName::from_str("  Checking  ").unwrap().as_str(),
      "Checking"
    );
    assert!(AccountName::from_str(&"a".repeat(128)).is_ok());
    assert!(AccountName::from_str(&"a".repeat(129)).is_err());

    // IdempotencyKey bounds: 1..=128
    assert!(IdempotencyKey::from_str("").is_err());
    assert_eq!(
      IdempotencyKey::from_str("  txn-uuid-1234  ")
        .unwrap()
        .as_str(),
      "txn-uuid-1234"
    );
    assert!(IdempotencyKey::from_str(&"k".repeat(128)).is_ok());
    assert!(IdempotencyKey::from_str(&"k".repeat(129)).is_err());
  }

  #[test]
  fn test_username_validation() {
    // Valid: 3..=64 characters, alphanumeric and underscore
    assert!(Username::try_from("alice_01".to_string()).is_ok());
    assert_eq!(
      Username::try_from("  bob_smith  ".to_string())
        .unwrap()
        .as_str(),
      "bob_smith"
    );

    // Invalid lengths
    assert!(Username::try_from("ab".to_string()).is_err());
    assert!(Username::try_from("a".repeat(65)).is_err());

    // Invalid characters
    assert!(Username::try_from("alice-smith".to_string()).is_err());
    assert!(Username::try_from("alice@mail".to_string()).is_err());
    assert!(Username::try_from("alice space".to_string()).is_err());
  }

  #[test]
  fn test_password_validation_and_exposure() {
    // Valid lengths: 8..=128 characters
    let valid_pw = "pass1234".to_string();
    let parsed = Password::try_from(valid_pw.clone()).expect("valid 8-char password");
    assert_eq!(parsed.expose(), "pass1234");

    // Invalid: under 8 characters
    assert!(Password::try_from("short12".to_string()).is_err());

    // Invalid: over 128 characters
    assert!(Password::try_from("x".repeat(129)).is_err());
  }

  #[test]
  fn test_fingerprint_deterministic_and_matches() {
    let cmd1 = TransferCmd {
      source_account_id: 1,
      destination_account_id: 2,
      amount: Amount::try_from(dec!(50.00)).unwrap(),
      description: Note::from_str("Settlement").unwrap(),
    };

    let cmd2 = TransferCmd {
      source_account_id: 1,
      destination_account_id: 2,
      amount: Amount::try_from(dec!(50.01)).unwrap(), // different amount
      description: Note::from_str("Settlement").unwrap(),
    };

    let fp1 = Fingerprint::of("transfer", &cmd1);
    let fp1_dup = Fingerprint::of("transfer", &cmd1);
    let fp2 = Fingerprint::of("transfer", &cmd2);

    assert_eq!(fp1, fp1_dup);
    assert_ne!(fp1, fp2);
    assert!(fp1.matches(fp1_dup.as_bytes()));
    assert!(!fp1.matches(fp2.as_bytes()));
  }

  #[test]
  fn test_entries_invariants_and_sorting() {
    let amt = Amount::try_from(dec!(100.00)).unwrap();

    // Outflow is negative, inflow is positive
    let out_entry = NewEntry::outflow(3, amt);
    let in_entry = NewEntry::inflow(1, amt);
    assert_eq!(out_entry.amount, dec!(-100.00));
    assert_eq!(in_entry.amount, dec!(100.00));

    // Valid transfer entry creation
    let entries = Entries::transfer(3, 1, amt).expect("valid transfer");
    assert_eq!(entries.as_slice().len(), 2);

    // Account IDs sorted unstably
    assert_eq!(entries.account_ids(), vec![1, 3]);

    // Error: fewer than 2 postings
    assert!(matches!(
      Entries::new(vec![out_entry.clone()]),
      Err(Error::InsufficientPostings(1))
    ));

    // Error: zero amount posting
    let zero_entry = NewEntry {
      account_id: 2,
      amount: Decimal::ZERO,
    };
    assert!(matches!(
      Entries::new(vec![zero_entry, in_entry.clone()]),
      Err(Error::ZeroAmountPosting)
    ));

    // Error: duplicate account ID
    let dup_entries = vec![NewEntry::outflow(1, amt), NewEntry::inflow(1, amt)];
    assert!(matches!(
      Entries::new(dup_entries),
      Err(Error::DuplicateAccountPosting)
    ));

    // Error: unbalanced entries
    let unbalanced = vec![
      NewEntry::outflow(1, amt),
      NewEntry {
        account_id: 2,
        amount: dec!(99.99),
      },
    ];
    assert!(matches!(
      Entries::new(unbalanced),
      Err(Error::UnbalancedTransaction(_))
    ));
  }
}
