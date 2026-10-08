use std::sync::OnceLock;
use std::time::Instant;
use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use metrics::{counter, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use crate::app::AppState;

static METRICS_HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

pub fn setup() -> PrometheusHandle {
	METRICS_HANDLE
		.get_or_init(|| {
			PrometheusBuilder::new()
				.install_recorder()
				.expect("failed to install Prometheus recorder")
		})
		.clone()
}

pub async fn track(req: Request, next: Next) -> Response {
	let start = Instant::now();
	
	let path = req
		.extensions()
		.get::<MatchedPath>()
		.map(|p| p.as_str().to_string())
		.unwrap_or_else(|| "unmatched".to_string());

	let method = req.method().clone();
	let response = next.run(req).await;
	let latency = start.elapsed().as_secs_f64();
	let status = response.status().as_u16().to_string();

	let labels = [
		("method", method.to_string()),
		("path", path),
		("status", status),
	];

	counter!("http_requests_total", &labels).increment(1);
	histogram!("http_request_duration_seconds", &labels).record(latency);

	response
}

pub async fn handler(
	State(state): State<AppState>
) -> String {
	state.prometheus.render()
}
