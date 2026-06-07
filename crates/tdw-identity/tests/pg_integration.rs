//! Postgres integration tests for [`PgUserStore`].
//!
//! These tests are gated by the `TDW_POSTGRES_TEST_URL` environment variable
//! and skip silently when it is absent, keeping the offline workspace test
//! suite unaffected.

#![cfg(feature = "postgres")]

use tdw_identity::{IdentityError, NewUser, PgUserStore, UserStore};

const NOW_MS: i64 = 1_749_254_400_000;

fn new_user(email: &str, password: &str) -> NewUser {
    NewUser {
        email: email.to_string(),
        password: password.to_string(),
        display_name: "PG Test User".to_string(),
    }
}

/// Return the test database URL or skip the test.
macro_rules! pg_url_or_skip {
    () => {
        match std::env::var("TDW_POSTGRES_TEST_URL") {
            Ok(url) => url,
            Err(_) => {
                eprintln!("TDW_POSTGRES_TEST_URL not set — skipping Postgres integration test");
                return;
            }
        }
    };
}

#[tokio::test]
async fn pg_register_and_authenticate() {
    let url = pg_url_or_skip!();
    let store = PgUserStore::connect(&url).await.expect("connect");

    let id = format!(
        "test-pg-auth-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    let email = format!("pg-auth-{}@example.com", &id[..10]);

    let user = store
        .register(new_user(&email, "securepass1"), id.clone(), NOW_MS)
        .await
        .expect("register");

    assert_eq!(user.id, id);
    assert_eq!(user.email, email);

    let auth_user = store
        .authenticate(&email, "securepass1")
        .await
        .expect("authenticate");
    assert_eq!(auth_user.id, id);
}

#[tokio::test]
async fn pg_duplicate_email_returns_email_taken() {
    let url = pg_url_or_skip!();
    let store = PgUserStore::connect(&url).await.expect("connect");

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let email = format!("pg-dup-{ts}@example.com");

    store
        .register(
            new_user(&email, "password01"),
            format!("pg-dup-1-{ts}"),
            NOW_MS,
        )
        .await
        .expect("first register");

    let result = store
        .register(
            new_user(&email, "password02"),
            format!("pg-dup-2-{ts}"),
            NOW_MS,
        )
        .await;
    assert!(matches!(result, Err(IdentityError::EmailTaken)));
}

#[tokio::test]
async fn pg_wrong_password_returns_invalid_credentials() {
    let url = pg_url_or_skip!();
    let store = PgUserStore::connect(&url).await.expect("connect");

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let email = format!("pg-wrongpw-{ts}@example.com");

    store
        .register(
            new_user(&email, "correctpass"),
            format!("pg-wp-{ts}"),
            NOW_MS,
        )
        .await
        .expect("register");

    let result = store.authenticate(&email, "wrongpass").await;
    assert!(matches!(result, Err(IdentityError::InvalidCredentials)));
}

#[tokio::test]
async fn pg_get_by_id_and_email() {
    let url = pg_url_or_skip!();
    let store = PgUserStore::connect(&url).await.expect("connect");

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let email = format!("pg-get-{ts}@example.com");
    let id = format!("pg-get-{ts}");

    store
        .register(new_user(&email, "getbytest"), id.clone(), NOW_MS)
        .await
        .expect("register");

    let by_id = store.get_by_id(&id).await.expect("get_by_id");
    assert_eq!(by_id.email, email);

    let by_email = store.get_by_email(&email).await.expect("get_by_email");
    assert_eq!(by_email.id, id);
}

#[tokio::test]
async fn pg_touch_last_active() {
    let url = pg_url_or_skip!();
    let store = PgUserStore::connect(&url).await.expect("connect");

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let email = format!("pg-touch-{ts}@example.com");
    let id = format!("pg-touch-{ts}");

    store
        .register(new_user(&email, "touchtest"), id.clone(), NOW_MS)
        .await
        .expect("register");

    let new_ts = NOW_MS + 50_000;
    store.touch_last_active(&id, new_ts).await.expect("touch");

    let user = store.get_by_id(&id).await.expect("get");
    assert_eq!(user.last_active_at_ms, Some(new_ts));
}
