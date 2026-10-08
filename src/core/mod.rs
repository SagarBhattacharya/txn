use crate::core::errors::Error;
use envconfig::Envconfig;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

pub mod auth;
pub mod errors;
pub mod ledger;
pub mod types;

pub fn init_tracing() {
  let filter = EnvFilter::try_from_default_env()
    .unwrap_or_else(|_| EnvFilter::new("info,txn=debug,tower_http=debug,sqlx=warn"));

  let formatting_layer = tracing_subscriber::fmt::layer()
    .with_target(false)
    .compact();

  let _ = Registry::default()
    .with(filter)
    .with(formatting_layer)
    .try_init();
}

// --------------- CONFIG -----------------

#[derive(Debug, Clone, Envconfig)]
pub struct Config {
  #[envconfig(from = "DATABASE_URL")]
  pub database_url: String,

  #[envconfig(from = "MAX_DB_CONNECTIONS")]
  pub max_db_connections: u32,

  #[envconfig(from = "SERVER_PORT", default = "8080")]
  pub server_port: u16,

  #[envconfig(from = "JWT_SECRET_KEY")]
  pub jwt_secret: String,
}

impl Config {
  pub fn init() -> Result<Self, Error> {
    dotenvy::dotenv().ok();
    let cfg = Self::init_from_env().map_err(|e| Error::Internal(e.to_string()))?;

    if cfg.jwt_secret.trim().len() < 32 {
      return Err(Error::Internal(
        "JWT_SECRET_KEY must be at least 32 characters \
        long for cryptographic security"
          .into(),
      ));
    }

    Ok(cfg)
  }
}
