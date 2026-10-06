//! A pool's wait limit: a statement that finds every connection busy fails once it has waited
//! that long, rather than queueing for as long as it takes (the default), and runs again as
//! soon as a connection is free.

use std::time::{Duration, Instant};

use ash_core::Error;
use ash_postgres::{PoolSettings, Postgres};

fn config() -> tokio_postgres::Config {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".into());
    url.parse().expect("a DATABASE_URL")
}

#[tokio::test]
async fn a_statement_fails_once_it_has_waited_the_limit() {
    let settings = PoolSettings { size: 1, wait_timeout: Some(Duration::from_millis(150)) };
    let db = Postgres::connect_with_pool(config(), settings).await.unwrap();

    // The pool's only connection is out.
    let held = db.pool().unwrap().get().await.unwrap();

    let started = Instant::now();
    let err = db.execute_sql("SELECT 1").await.unwrap_err();
    let waited = started.elapsed();
    match err {
        Error::DataLayer(message) => assert!(message.contains("pool exhausted"), "{message}"),
        other => panic!("expected a pool error, got {other:?}"),
    }
    assert!(waited >= Duration::from_millis(140), "gave up after {waited:?}, before the limit");
    assert!(waited < Duration::from_secs(2), "waited {waited:?}, far past the limit");

    // A connection free again, the next statement runs.
    drop(held);
    db.execute_sql("SELECT 1").await.unwrap();
}

#[tokio::test]
async fn with_no_limit_a_statement_waits_for_a_connection() {
    let settings = PoolSettings { size: 1, wait_timeout: None };
    let db = Postgres::connect_with_pool(config(), settings).await.unwrap();
    let held = db.pool().unwrap().get().await.unwrap();

    let waiting = {
        let db = db.clone();
        tokio::spawn(async move { db.execute_sql("SELECT 1").await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!waiting.is_finished(), "a statement with no wait limit gave up");
    drop(held);
    waiting.await.unwrap().unwrap();
}
