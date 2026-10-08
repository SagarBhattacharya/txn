use crate::core::errors::AppResult;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

mod queries;
pub mod rows;

pub use queries::*;
use crate::core::Config;

pub async fn connect(config: &Config) -> AppResult<PgPool> {
  let pool = PgPoolOptions::new()
    .max_connections(config.max_db_connections)
    .connect(config.database_url.as_str())
    .await?;

  Ok(pool)
}

pub async fn ping(pool: &PgPool) -> AppResult<()> {
  sqlx::query("select 1").execute(pool).await?;
  Ok(())
}
