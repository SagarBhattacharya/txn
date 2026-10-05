use crate::LedgerResult;
use crate::config::Config;
use crate::db::Repo;
use axum::Router;
use tokio::net::TcpListener;

pub mod account;
pub mod transaction;

#[derive(Debug, Clone)]
pub struct AppState {
  pub repo: Repo,
}

impl AppState {
  pub fn new(repo: Repo) -> Self {
    Self { repo }
  }
}

pub struct AppRouter;

impl AppRouter {
  pub fn with_state(state: AppState) -> Router {
    Router::new()
      .nest("/accounts", account::account_router())
      .nest("/transactions", transaction::transaction_router())
      .with_state(state)
  }
}

pub struct App {
  port: u16,
  router: Router,
}

impl App {
  pub async fn new(config: Config) -> LedgerResult<Self> {
    let repo = Repo::new(&config).await?;
    let state = AppState::new(repo);

    Ok(Self {
      router: AppRouter::with_state(state),
      port: config.server_port,
    })
  }

  pub async fn run(self) -> LedgerResult<()> {
    let listener = TcpListener::bind(("127.0.0.1", self.port)).await?;

    println!("Listening on http://127.0.0.1:{}", self.port);
    axum::serve(listener, self.router).await?;
    Ok(())
  }
}
