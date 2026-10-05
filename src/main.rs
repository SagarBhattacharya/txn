use txn::LedgerResult;
use txn::api::App;
use txn::config::Config;

#[tokio::main]
async fn main() -> LedgerResult<()> {
  App::new(Config::init()?).await?.run().await
}
