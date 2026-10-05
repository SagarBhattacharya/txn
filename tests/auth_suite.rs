mod common;

use axum::http::StatusCode;
use common::TestHarness;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test]
async fn test_auth_registration_lifecycle(pool: PgPool) {
	let harness = TestHarness::new(pool).await;

	// 1. Successful Registration
	let (status, body) = harness
		.post_unauthenticated(
			"/auth/register",
			json!({
        "username": "alice_ledger",
        "password": "supersecurepassword123"
      }),
		)
		.await;

	assert_eq!(status, StatusCode::CREATED);
	assert_eq!(body["username"], "alice_ledger");
	assert!(body["token"].is_string());
	assert!(body["user_id"].is_i64());

	// 2. Duplicate Username Rejection (Conflict 409)
	let (dup_status, dup_body) = harness
		.post_unauthenticated(
			"/auth/register",
			json!({
        "username": "alice_ledger",
        "password": "anotherpassword123"
      }),
		)
		.await;

	assert_eq!(dup_status, StatusCode::CONFLICT);
	assert_eq!(dup_body["error"], "Username already taken");
}

#[sqlx::test]
async fn test_auth_registration_validation(pool: PgPool) {
	let harness = TestHarness::new(pool).await;

	// Short password (< 8 chars)
	let (status, _) = harness
		.post_unauthenticated(
			"/auth/register",
			json!({
        "username": "valid_user",
        "password": "123"
      }),
		)
		.await;
	assert_eq!(status, StatusCode::BAD_REQUEST);

	// Invalid username characters
	let (status, _) = harness
		.post_unauthenticated(
			"/auth/register",
			json!({
        "username": "bad user@name!",
        "password": "validpassword123"
      }),
		)
		.await;
	assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test]
async fn test_auth_login_lifecycle(pool: PgPool) {
	let harness = TestHarness::new(pool).await;

	// 1. Register User
	let (reg_status, _) = harness
		.post_unauthenticated(
			"/auth/register",
			json!({
        "username": "bob_trader",
        "password": "strongpassword123"
      }),
		)
		.await;
	assert_eq!(reg_status, StatusCode::CREATED);

	// 2. Login with Valid Credentials
	let (login_status, login_body) = harness
		.post_unauthenticated(
			"/auth/login",
			json!({
        "username": "bob_trader",
        "password": "strongpassword123"
      }),
		)
		.await;
	assert_eq!(login_status, StatusCode::OK);
	assert_eq!(login_body["username"], "bob_trader");
	assert!(login_body["token"].is_string());

	// 3. Login with Invalid Password
	let (fail_status, _) = harness
		.post_unauthenticated(
			"/auth/login",
			json!({
        "username": "bob_trader",
        "password": "wrongpassword123"
      }),
		)
		.await;
	assert_eq!(fail_status, StatusCode::UNAUTHORIZED);

	// 4. Login with Non-Existent Username (Timing attack mitigation path)
	let (fail_status, _) = harness
		.post_unauthenticated(
			"/auth/login",
			json!({
        "username": "ghost_user",
        "password": "somepassword123"
      }),
		)
		.await;
	assert_eq!(fail_status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test]
async fn test_health_endpoints(pool: PgPool) {
	let harness = TestHarness::new(pool).await;

	// Liveness probe (process is running)
	let (status, body) = harness.get_unauthenticated("/health/live").await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["status"], "alive");

	// Readiness probe (PostgreSQL connection is healthy)
	let (status, body) = harness.get_unauthenticated("/health/ready").await;
	assert_eq!(status, StatusCode::OK);
	assert_eq!(body["status"], "ready");
}