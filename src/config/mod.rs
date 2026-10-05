use envconfig::{Envconfig, Error};

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
    Self::init_from_env()
  }
}
