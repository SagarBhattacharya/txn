use crate::LedgerResult;
use crate::config::Config;
use crate::db::Repo;
use axum::{middleware, Router};
use tokio::net::TcpListener;
use crate::auth::middleware::auth_middleware;

pub mod account;
pub mod transaction;
pub mod auth;
pub mod health;
mod extractors;

#[derive(Debug, Clone)]
pub struct AppState {
  pub jwt_secret: String,
  pub repo: Repo,
}

impl AppState {
  pub fn new(
    repo: Repo,
    jwt_secret: String,
  ) -> Self {
    Self { repo, jwt_secret }
  }
}

pub struct AppRouter;

impl AppRouter {
  pub fn with_state(state: AppState) -> Router {
    let public_routes = Router::new()
      .nest("/health", health::health_router())
      .nest("/auth", auth::auth_router());

    // Protected ledger engine routes
    let protected_ledger = Router::new()
      .nest("/accounts", account::account_router())
      .nest("/transactions", transaction::transaction_router())
      .route_layer(middleware::from_fn_with_state(
        state.clone(),
        auth_middleware,
      ));

    // Combine both sets of routes
    Router::new()
      .merge(public_routes)
      .merge(protected_ledger)
      .with_state(state)
  }
}

pub struct App {
  port: u16,
  router: Router,
}

impl App {
  pub async fn new(config: Config) -> LedgerResult<Self> {
    let port = config.server_port;
    let repo = Repo::new(&config).await?;
    let state = AppState::new(repo, config.jwt_secret);

    Ok(Self {
      router: AppRouter::with_state(state),
      port,
    })
  }

  pub async fn run(self) -> LedgerResult<()> {
    let listener = TcpListener::bind(("127.0.0.1", self.port)).await?;

    println!("Listening on http://127.0.0.1:{}", self.port);
    axum::serve(listener, self.router).await?;
    Ok(())
  }
}