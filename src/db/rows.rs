use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Type};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Deserialize, Serialize)]
#[sqlx(type_name = "account_type", rename_all = "lowercase")]
pub enum AccountType {
  Asset,
  Liability,
  Equity,
  Revenue,
  Expense,
}

impl AccountType {
  pub fn allows_negative_balance(&self) -> bool {
    !matches!(self, AccountType::Asset)
  }
}

#[derive(Debug, FromRow, PartialEq)]
pub struct User {
  pub id: i32,
  pub username: String,
  pub password_hash: String,
  pub created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow, PartialEq, Serialize, Deserialize)]
pub struct Account {
  pub id: i32,
  pub owner_id: i32,
  pub name: String,
  pub account_type: AccountType,
  pub created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow, PartialEq)]
pub struct Transaction {
  pub id: i32,
  pub user_id: i32,
  pub description: String,
  pub idempotency_key: Option<String>,
  pub request_hash: Vec<u8>,
  pub reversed_transaction_id: Option<i32>,
  pub created_at: DateTime<Utc>,
}

#[derive(Debug, PartialEq, FromRow)]
pub struct EntryWithOwner {
  pub account_id: i32,
  pub amount: Decimal,
  pub owner_id: i32,
}
