//! Integration test for the Postgres-backed alert store.
//!
//! Runs only with `--features postgres`; if `TDW_POSTGRES_TEST_URL` is not set,
//! the test compiles and exits without touching a database.

#![cfg(feature = "postgres")]

use tdw_alerts::{AlertDirection, AlertStore, NewAlert, PgAlertStore, PriceAlert};

fn database_url() -> Option<String> {
    std::env::var("TDW_POSTGRES_TEST_URL").ok()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as i64
}

fn test_prefix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!("tdw_alert_pg_test_{}_{}_", std::process::id(), nanos)
}

async fn cleanup(store: &PgAlertStore, ids: &[String]) {
    for id in ids {
        let _ = store.delete_by_id(id).await;
    }
}

#[tokio::test]
async fn pg_alert_store_full_surface_roundtrip() {
    let Some(url) = database_url() else {
        eprintln!("TDW_POSTGRES_TEST_URL not set; skipping pg alert integration test");
        return;
    };

    let store = PgAlertStore::connect(&url)
        .await
        .unwrap_or_else(|error| panic!("postgres alert connect: {error}"));

    let prefix = test_prefix();
    let id1 = format!("{prefix}a1");
    let id2 = format!("{prefix}a2");
    let id3 = format!("{prefix}a3");
    cleanup(&store, &[id1.clone(), id2.clone(), id3.clone()]).await;

    let now = now_ms();

    let alert1 = PriceAlert::new(
        NewAlert {
            owner_id: format!("{prefix}owner"),
            symbol: "btc".to_string(),
            target_price: 50_000.0,
            condition: AlertDirection::Above,
            expires_at_ms: None,
        },
        id1.clone(),
        now,
    );
    // alert2 created 1s later so list_by_owner order is deterministic
    let alert2 = PriceAlert::new(
        NewAlert {
            owner_id: format!("{prefix}owner"),
            symbol: "eth".to_string(),
            target_price: 2_000.0,
            condition: AlertDirection::Below,
            expires_at_ms: None,
        },
        id2.clone(),
        now + 1_000,
    );
    // Different owner
    let alert3 = PriceAlert::new(
        NewAlert {
            owner_id: format!("{prefix}other"),
            symbol: "SP500".to_string(),
            target_price: 5_000.0,
            condition: AlertDirection::Above,
            expires_at_ms: None,
        },
        id3.clone(),
        now,
    );

    store
        .insert(&alert1)
        .await
        .unwrap_or_else(|error| panic!("insert a1: {error}"));
    store
        .insert(&alert2)
        .await
        .unwrap_or_else(|error| panic!("insert a2: {error}"));
    store
        .insert(&alert3)
        .await
        .unwrap_or_else(|error| panic!("insert a3: {error}"));

    // list_by_owner — newest-first, owner-scoped
    let owned = store
        .list_by_owner(&format!("{prefix}owner"))
        .await
        .unwrap_or_else(|error| panic!("list_by_owner: {error}"));
    assert_eq!(owned.len(), 2);
    assert_eq!(owned[0].id, id2, "newest first");
    assert_eq!(owned[1].id, id1);
    assert_eq!(owned[0].symbol, "ETH", "symbol normalized on insert");

    // list_eligible
    let eligible = store
        .list_eligible(now)
        .await
        .unwrap_or_else(|error| panic!("list_eligible: {error}"));
    assert!(
        eligible.iter().any(|a| a.id == id1),
        "a1 should be eligible"
    );

    // mark_fired — fire-once semantics
    let fired = store
        .mark_fired(&id1, now)
        .await
        .unwrap_or_else(|error| panic!("mark_fired first: {error}"));
    assert!(fired, "first fire should return true");

    let fired_again = store
        .mark_fired(&id1, now)
        .await
        .unwrap_or_else(|error| panic!("mark_fired second: {error}"));
    assert!(!fired_again, "second fire should return false");

    let after_fire = store
        .list_by_owner(&format!("{prefix}owner"))
        .await
        .unwrap_or_else(|error| panic!("list after fire: {error}"));
    let a1_state = after_fire.iter().find(|a| a.id == id1).expect("a1");
    assert!(a1_state.triggered);
    assert!(!a1_state.active);

    // set_active
    assert!(
        store
            .set_active(&id2, false)
            .await
            .unwrap_or_else(|error| panic!("set_active: {error}")),
        "set_active should return true"
    );
    let after_deactivate = store
        .list_by_owner(&format!("{prefix}owner"))
        .await
        .unwrap_or_else(|error| panic!("list after deactivate: {error}"));
    let a2_state = after_deactivate.iter().find(|a| a.id == id2).expect("a2");
    assert!(!a2_state.active);

    // delete_by_id
    assert!(
        store
            .delete_by_id(&id3)
            .await
            .unwrap_or_else(|error| panic!("delete a3: {error}")),
        "delete should return true"
    );
    assert!(
        !store
            .delete_by_id(&id3)
            .await
            .unwrap_or_else(|error| panic!("delete a3 again: {error}")),
        "second delete should return false"
    );

    cleanup(&store, &[id1, id2]).await;
}
