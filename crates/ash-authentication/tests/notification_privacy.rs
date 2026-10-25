use std::sync::{Arc, Mutex};

use ash_authentication::authentication;
use ash_core::{Notification, Resource, Runtime, SyncFnNotifier, Value};
use ash_memory::Memory;
use uuid::Uuid;

#[authentication]
ash_core::resource! {
User {
    table "event_users";

attributes {
    id: Uuid [pk];
    email: String;
}

authentication {
    strategy password {
        identity_field: email;
        hashed_password_field: hashed_password;
        min_password_length: 8;
        require_confirmation: true;
    }
}

actions {
    read read { primary; }
}
}}

async fn registration_privacy<D: ash_core::DataLayer>(runtime: Runtime<D>) {
    let heard: Arc<Mutex<Vec<Notification>>> = Arc::default();
    let sink = heard.clone();
    let notifier = SyncFnNotifier::new("capture", move |n: &Notification| {
        sink.lock().unwrap().push(n.clone());
        Ok(())
    });
    let ctx = runtime.bootstrap().with_notifier(Arc::new(notifier));

    let user: User = User::register_with_password(&ctx)
        .email("alice@example.com")
        .password("super-secret-password-123")
        .password_confirmation("super-secret-password-123")
        .await
        .unwrap();
    assert!(user.hashed_password.is_none());

    let heard = heard.lock().unwrap();
    let note = heard
        .iter()
        .find(|n| n.resource == User::DEF.name)
        .expect("a notification");
    assert_eq!(note.get("hashed_password"), Some(&Value::Null));
    assert!(note.record.is_forbidden("hashed_password"));
    assert!(note.metadata.is_empty());
    assert!(note.actor_id.is_none());
    assert!(!format!("{note:?}").contains("super-secret-password-123"));

    let plaintext = Value::from("super-secret-password-123");
    assert!(
        !note.metadata.values().any(|v| *v == plaintext),
        "notification metadata carries the plaintext password"
    );
}

#[tokio::test]
async fn register_notification_does_not_carry_the_plaintext_password() {
    registration_privacy(Runtime::new(Memory::new())).await;
}
#[tokio::test]
async fn sqlite_registration_notifications_hide_passwords_and_hashes() {
    let runtime = Runtime::new(ash_sqlite::Sqlite::memory().await.unwrap());
    runtime.install(&[&User::DEF]).await.unwrap();
    registration_privacy(runtime).await;
}
#[tokio::test]
async fn postgres_registration_notifications_hide_passwords_and_hashes() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL for Postgres tests");
    let runtime = Runtime::new(ash_postgres::Postgres::connect(&url).await.unwrap());
    runtime.install(&[&User::DEF]).await.unwrap();
    registration_privacy(runtime).await;
}
