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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
