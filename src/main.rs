use std::sync::Arc;
use txn::app::auth::JwtKeys;
use txn::app::{self, AppState};
use txn::core::config::Config;
use txn::core::errors::AppResult;
use txn::db;

#[tokio::main]
async fn main() -> AppResult<()> {
  let config = Config::init()?;
  let pool = db::connect(&config).await?;
  let jwt = Arc::new(JwtKeys::new(config.jwt_secret.as_ref()));

  let app_state = AppState::new(pool, jwt);
  app::serve(app_state, config.server_port).await
}
