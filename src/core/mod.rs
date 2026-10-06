use tracing_subscriber::{EnvFilter, Registry};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub mod config;
pub mod errors;
pub mod ledger;
pub mod types;

pub fn init_tracing() {
	let filter = EnvFilter::try_from_default_env()
		.unwrap_or_else(|_| EnvFilter::new(
			"info,txn=debug,tower_http=debug,sqlx=warn"
		));

	let formatting_layer = tracing_subscriber::fmt::layer()
		.with_target(false)
		.compact();

	let _ = Registry::default()
		.with(filter)
		.with(formatting_layer)
		.try_init();
}