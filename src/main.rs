use std::sync::Arc;
use txn::app::auth::JwtKeys;
use txn::app::{self, setup_metrics, AppState};
use txn::core::config::Config;
use txn::core::errors::{AppResult, Error};
use txn::db;

#[tokio::main]
async fn main() -> AppResult<()> {
  txn::core::init_tracing();
  let config = Config::init()?;

  let pool = db::connect(&config).await?;

  tracing::info!("running database migrations...");

  sqlx::migrate!("./migrations")
    .run(&pool)
    .await
    .map_err(|err| Error::Internal(err.to_string()))?;

  tracing::info!("database migrations applied successfully");

  let jwt = Arc::new(JwtKeys::new(config.jwt_secret.as_ref()));
  let prometheus = setup_metrics();
  let app_state = AppState::new(pool, jwt);

  app::serve(app_state, config.server_port, prometheus).await
}
