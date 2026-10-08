use std::sync::Arc;
use txn::*;

#[tokio::main]
async fn main() -> AppResult<()> {
  init_tracing();
  let config = Config::init()?;
  let pool = db::connect(&config).await?;

  tracing::info!("running database migrations...");

  sqlx::migrate!("./migrations")
    .run(&pool)
    .await
    .map_err(|err| Error::Internal(err.to_string()))?;

  tracing::info!("database migrations applied successfully");

  let jwt = Arc::new(JwtKeys::new(config.jwt_secret.as_ref()));
  let app_state = AppState::new(pool, jwt);
  let app = App::from_state(app_state)?;
  app.run(config.server_port).await
}