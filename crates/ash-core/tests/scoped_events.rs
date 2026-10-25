use std::sync::{Arc, Mutex};
use std::time::Duration;

use ash_core::{
    Actor, AttrType, DataLayer, MetadataDef, Notification, Resource, Runtime, ScopedNotifier,
    SyncFnNotifier, Value, resource,
};
use ash_memory::Memory;
use ash_pubsub::{ContextPubSubExt, PubSub};
use uuid::Uuid;

resource! {
    Account {
        table "scoped_event_accounts";
        multitenancy { strategy: attribute; attribute: org; }
        actor { role: String; }
        attributes {
            id: Uuid [pk]; org: String; owner_id: Uuid;
            name: String; secret: Option<String>;
        }
        policies {
            policy action_type(read) { authorize_if relates_to_actor(owner_id); }
            policy action_type(create) { authorize_if always; }
            policy action_type(update) { authorize_if always; }
            policy action_type(destroy) { authorize_if always; }
        }
        field_policies { field secret { authorize_if actor_attribute_equals(role, "admin"); } }
        actions {
            read read { primary; }
            create open {
                primary; accept [owner_id, name, secret]; argument invite_code: Option<String>;
                validate string_length(invite_code, min: 1);
            }
            update transfer { primary; accept [owner_id, name]; }
            destroy close { primary; }
        }
    }
}

fn capture() -> (Arc<Mutex<Vec<Notification>>>, Arc<dyn ash_core::Notifier>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let notifier = SyncFnNotifier::new("capture", move |event: &Notification| {
        sink.lock().unwrap().push(event.clone());
        Ok(())
    });
    (events, Arc::new(notifier))
}
async fn receive(subscription: &mut ash_pubsub::Subscription) -> Arc<Notification> {
    tokio::time::timeout(Duration::from_secs(3), subscription.recv())
        .await
        .expect("event within 3 seconds")
        .unwrap()
}
async fn silent(subscription: &mut ash_pubsub::Subscription) {
    assert!(
        tokio::time::timeout(Duration::from_millis(30), subscription.recv())
            .await
            .is_err()
    );
}
async fn check_projection<D: DataLayer + Send + Sync + 'static>(runtime: Runtime<D>) {
    let broker = Arc::new(PubSub::new());
    let runtime = runtime.with_pubsub(broker.clone());
    let owner = Uuid::new_v4();
    let clerk = runtime.bind(
        Some(
            Actor::new(owner)
                .with_role("clerk")
                .with("token", "subscriber-secret"),
        ),
        Some("a:*:b".into()),
    );
    let admin = runtime.bind(
        Some(Actor::new(owner).with_role("admin")),
        Some("a:*:b".into()),
    );
    let foreign = runtime.bind(Some(Actor::new(owner).with_role("admin")), Some("a".into()));
    let denied = runtime.bind(
        Some(Actor::new(Uuid::new_v4()).with_role("admin")),
        Some("a:*:b".into()),
    );
    let mut clerk_events = broker.subscribe(&clerk, &Account::DEF, "*").unwrap();
    let mut admin_events = broker.subscribe(&admin, &Account::DEF, "*").unwrap();
    let mut foreign_events = broker.subscribe(&foreign, &Account::DEF, "*").unwrap();
    let mut denied_events = broker.subscribe(&denied, &Account::DEF, "*").unwrap();
    assert_eq!(clerk_events.topic_key().tenant.as_deref(), Some("a:*:b"));
    assert!(!format!("{broker:?}").contains("subscriber-secret"));
    let writer = runtime
        .bind(
            Some(Actor::new(Uuid::new_v4()).with("api_token", "writer-token")),
            Some("a:*:b".into()),
        )
        .with_metadata("request_id", "r1")
        .with_metadata("private_context", "private");
    let (ordinary, notifier) = capture();
    let reader = clerk
        .event_reader(&Account::DEF, None)
        .unwrap()
        .with_metadata(&[MetadataDef::new("request_id", AttrType::String)]);
    let writer = writer.with_notifier(Arc::new(ScopedNotifier::new(reader, notifier)));
    let written = Account::open(&writer)
        .owner_id(owner)
        .name("account")
        .secret(Some("hidden-value".into()))
        .invite_code(Some("invite-secret".into()))
        .await
        .unwrap();
    assert!(written.secret.is_none());
    let clerk_event = receive(&mut clerk_events).await;
    let admin_event = receive(&mut admin_events).await;
    assert_eq!(clerk_event.get("secret"), Some(&Value::Null));
    assert!(clerk_event.record.is_forbidden("secret"));
    assert_eq!(
        admin_event.get("secret"),
        Some(&Value::from("hidden-value"))
    );
    assert!(clerk_event.actor_id.is_none());
    assert!(clerk_event.metadata.is_empty());
    let serialized = format!("{:?}", clerk_event);
    for secret in [
        "writer-token",
        "invite-secret",
        "private_context",
        "hidden-value",
    ] {
        assert!(!serialized.contains(secret));
    }
    {
        let events = ordinary.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].metadata.len(), 1);
        assert_eq!(events[0].metadata["request_id"], Value::from("r1"));
        assert_eq!(events[0].get("secret"), Some(&Value::Null));
    }
    silent(&mut foreign_events).await;
    silent(&mut denied_events).await;
    // A prior owner cannot receive a transfer, and the new owner cannot read its preimage.
    let new_owner = denied.actor().unwrap().id;
    let moved = written
        .transfer(&writer)
        .owner_id(new_owner)
        .name("transferred")
        .await
        .unwrap();
    silent(&mut clerk_events).await;
    silent(&mut admin_events).await;
    let transferred = receive(&mut denied_events).await;
    assert!(transferred.previous.is_none());
    moved.close(&writer).await.unwrap();
    assert_eq!(receive(&mut denied_events).await.action, "close");
    silent(&mut clerk_events).await;
}

#[tokio::test]
async fn memory_events_use_reader_fields_metadata_and_tenant_admission() {
    check_projection(Runtime::new(Memory::new())).await;
}
#[tokio::test]
async fn sqlite_events_use_reader_fields_metadata_and_tenant_admission() {
    let runtime = Runtime::new(ash_sqlite::Sqlite::memory().await.unwrap());
    runtime.install(&[&Account::DEF]).await.unwrap();
    check_projection(runtime).await;
}

#[tokio::test]
async fn notifier_grants_disclose_only_declared_fields_and_cannot_cross_runtimes() {
    let runtime = Runtime::new(Memory::new());
    let (events, sink) = capture();
    let grant = runtime
        .notifier_grant::<Account>(&["name", "secret"])
        .unwrap()
        .with_actor_id()
        .with_metadata(&[MetadataDef::new("request_id", AttrType::String)]);
    let notifier = grant.attach(sink);
    let actor = Actor::new(Uuid::new_v4()).with("token", "private-actor-token");
    let writer = runtime
        .bind(Some(actor.clone()), Some("b".into()))
        .with_metadata("request_id", "r2")
        .with_metadata("other", "private-context")
        .with_notifier(notifier.clone());
    Account::open(&writer)
        .owner_id(Uuid::new_v4())
        .name("visible-to-mail")
        .secret(Some("granted-value".into()))
        .invite_code(Some("never-an-event".into()))
        .await
        .unwrap();
    {
        let events = events.lock().unwrap();
        let event = &events[0];
        assert_eq!(event.record.fields().len(), 2);
        assert_eq!(event.get("secret"), Some(&Value::from("granted-value")));
        assert_eq!(event.actor_id, Some(actor.id));
        assert_eq!(event.metadata.len(), 1);
        assert!(!format!("{event:?}").contains("never-an-event"));
        assert!(!format!("{event:?}").contains("private-actor-token"));
    }
    let foreign = Runtime::new(Memory::new())
        .bind(None, Some("b".into()))
        .with_notifier(notifier);
    Account::open(&foreign)
        .owner_id(Uuid::new_v4())
        .name("foreign-runtime")
        .await
        .unwrap();
    assert_eq!(events.lock().unwrap().len(), 1);
    assert!(runtime.notifier_grant::<Account>(&["not_a_field"]).is_err());
}

#[test]
fn subscription_scope_keys_keep_authority_backend_tenant_and_system_binding() {
    let runtime = Runtime::new(Memory::new());
    let a = runtime.bind(None, Some("a".into()));
    assert_eq!(
        a.subscription_scope(),
        runtime.bind(None, Some("a".into())).subscription_scope()
    );
    assert_ne!(
        a.subscription_scope(),
        runtime.bind(None, Some("b".into())).subscription_scope()
    );
    assert_ne!(
        a.subscription_scope(),
        runtime
            .system()
            .bind(None, Some("a".into()))
            .subscription_scope()
    );
    assert_ne!(
        a.subscription_scope(),
        Runtime::new(Memory::new())
            .bind(None, Some("a".into()))
            .subscription_scope()
    );
    let member = runtime.bind(Some(Actor::new(Uuid::new_v4())), Some("a".into()));
    assert_eq!(
        member.subscription_scope(),
        member.clone().subscription_scope()
    );
    assert_ne!(
        member.subscription_scope(),
        runtime
            .bind(member.actor().cloned(), Some("a".into()))
            .subscription_scope()
    );
}

static READ_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
fn current_read() -> ash_core::TrustedExpr {
    ash_core::TrustedExpr::Filter(if READ_ENABLED.load(std::sync::atomic::Ordering::SeqCst) {
        ash_core::Filter::True
    } else {
        ash_core::Filter::False
    })
}
static LIVE_ACTIONS: &[ash_core::ActionDef] = &[
    ash_core::ActionDef::read("read")
        .preparations(&[ash_core::PreparationDef::Filter(current_read)]),
    ash_core::ActionDef::create("create").accept(&["owner_id", "name"]),
];
static LIVE_DEF: ash_core::ResourceDef = ash_core::ResourceDef {
    name: "LiveAccount",
    table: "live_event_accounts",
    actions: LIVE_ACTIONS,
    ..Account::DEF
};
#[tokio::test]
async fn event_admission_rechecks_the_default_read_on_every_event() {
    let broker = Arc::new(PubSub::new());
    let runtime = Runtime::new(Memory::new()).with_pubsub(broker.clone());
    let owner = Uuid::new_v4();
    let ctx = runtime.bind(Some(Actor::new(owner)), Some("a".into()));
    READ_ENABLED.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut events = broker.subscribe(&ctx, &LIVE_DEF, "*").unwrap();
    for enabled in [false, true] {
        READ_ENABLED.store(enabled, std::sync::atomic::Ordering::SeqCst);
        ash_core::create_dynamic(
            &ctx,
            &LIVE_DEF,
            &LIVE_ACTIONS[1],
            [
                ("owner_id".into(), owner.into()),
                ("name".into(), "changed".into()),
            ]
            .into_iter()
            .collect(),
        )
        .await
        .unwrap();
        if enabled {
            assert_eq!(
                receive(&mut events).await.get("name"),
                Some(&Value::from("changed"))
            );
        } else {
            silent(&mut events).await;
        }
    }
}

#[tokio::test]
async fn derived_registry_runtimes_retain_scoped_event_authority() {
    let memory = Memory::new();
    let broker = Arc::new(PubSub::new());
    let runtime = Runtime::new(memory.clone()).with_pubsub(broker.clone());
    let registry = runtime.with_backend(ash_core::StoreRegistry::new().with_default(memory));
    let owner = Uuid::new_v4();
    let subscriber = registry.bind(Some(Actor::new(owner).with_role("clerk")), Some("a".into()));
    let mut events = broker.subscribe(&subscriber, &Account::DEF, "*").unwrap();
    Account::open(&runtime.bind(None, Some("a".into())))
        .name("fleet event")
        .owner_id(owner)
        .await
        .unwrap();
    assert_eq!(
        receive(&mut events).await.get("name"),
        Some(&Value::from("fleet event"))
    );
    let unrelated = Runtime::new(Memory::new()).bind(Some(Actor::new(owner)), Some("a".into()));
    let mut unrelated_events = broker.subscribe(&unrelated, &Account::DEF, "*").unwrap();
    Account::open(&runtime.bind(None, Some("a".into())))
        .name("same application")
        .owner_id(owner)
        .await
        .unwrap();
    assert_eq!(
        receive(&mut events).await.get("name"),
        Some(&Value::from("same application"))
    );
    silent(&mut unrelated_events).await;
}
