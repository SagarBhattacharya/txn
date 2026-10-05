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

#[derive(Debug, FromRow, PartialEq)]
pub struct User {
  pub id: i32,
  pub username: String,
  pub password_hash: String,
  pub created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow, PartialEq)]
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
  pub description: String,
  pub idempotency_key: Option<String>,
  pub reversed_transaction_id: Option<i32>,
  pub created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow, PartialEq)]
pub struct Entry {
  pub id: i32,
  pub transaction_id: i32,
  pub account_id: i32,
  pub amount: Decimal,
}

#[derive(Debug, PartialEq)]
pub struct LockedAccountRecord {
  pub id: i32,
  pub account_type: AccountType,
}