use std::sync::Arc;
use std::time::Duration;

use ash_core::{Actor, Resource, resource};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_pubsub::{ContextPubSubExt, PubSub};
use async_graphql::Request;
use futures_util::StreamExt;
use serde_json::{Value as Json, json};
use uuid::Uuid;

pub mod owned {
    use super::*;

    resource! {
        Owned {
            table "event_owned";

            attributes {
                id: Uuid [pk];
                title: String;
                owner_id: Uuid;
                status: String;
            }

            policies {
                policy action_type(create) { authorize_if always; }
                policy action_type(destroy) { authorize_if always; }
                policy action_type(read) { authorize_if relates_to_actor(owner_id); }
            }

            actions {
                create create { primary; accept [title, owner_id, status]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}

pub mod team {
    use super::*;

    resource! {
        Team {
            table "event_teams";

            attributes {
                id: Uuid [pk];
                name: String;
            }

            relationships {
                has_many tasks: super::task::Task [fk: team_id];
            }

            actions {
                create create { primary; accept [name]; }
                read read { primary; }
            }
        }
    }
}

pub mod task {
    use super::*;

    resource! {
        Task {
            table "event_tasks";

            attributes {
                id: Uuid [pk];
                team_id: Uuid;
                title: String;
            }

            relationships {
                belongs_to team: super::team::Team [fk: team_id];
            }

            actions {
                create create { primary; accept [team_id, title]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}

pub mod by_attr {
    use super::*;

    resource! {
        Shelf {
            table "event_shelves";

            multitenancy {
                strategy: attribute;
                attribute: org;
            }

            attributes {
                id: Uuid [pk];
                org: String;
                label: String;
            }

            actions {
                create create { primary; accept [label]; }
                read read { primary; }
            }
        }
    }
}

pub mod by_context {
    use super::*;

    resource! {
        Vault {
            table "event_vaults";

            multitenancy {
                strategy: context;
            }

            attributes {
                id: Uuid [pk];
                label: String;
            }

            actions {
                create create { primary; accept [label]; }
                read read { primary; }
            }
        }
    }
}

use by_attr::Shelf;
use by_context::Vault;
use owned::Owned;
use task::Task;
use team::Team;

async fn next_event(
    stream: &mut (impl futures_util::Stream<Item = async_graphql::Response> + Unpin),
) -> Option<Json> {
    tokio::time::timeout(Duration::from_millis(300), stream.next())
        .await
        .ok()
        .flatten()
        .map(|response| {
            assert!(response.errors.is_empty(), "{:?}", response.errors);
            response.data.into_json().unwrap()
        })
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(50)).await;
}

/// Destroy admission uses the protected snapshot after the row has been deleted.
#[tokio::test]
async fn destroy_event_skips_subscribers_who_cannot_read_the_record() {
    let pubsub = PubSub::new();
    let runtime = ash_core::Runtime::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let base = runtime.bootstrap();
    let schema = AshGraphQL::from_resources(&[&Owned::DEF])
        .with_pubsub(pubsub)
        .finish::<Memory>()
        .unwrap();
    let viewer = Actor::new(Uuid::new_v4());
    let other = Uuid::new_v4();

    let theirs = Owned::create(&base)
        .title("theirs")
        .owner_id(other)
        .status("open")
        .await
        .unwrap();
    let mine = Owned::create(&base)
        .title("mine")
        .owner_id(viewer.id)
        .status("open")
        .await
        .unwrap();

    // Control: the viewer's list query.
    let listed = schema
        .execute(
            Request::new("{ listOwneds { title } }").data(runtime.bind(Some(viewer.clone()), None)),
        )
        .await;
    assert!(listed.errors.is_empty(), "{:?}", listed.errors);
    println!(
        "control list as viewer: {}",
        listed.data.into_json().unwrap()
    );

    let mut stream = schema.execute_stream(
        Request::new("subscription { ownedDestroyed { destroyed } }")
            .data(runtime.bind(Some(viewer.clone()), None)),
    );
    assert!(next_event(&mut stream).await.is_none());
    settle().await;

    theirs.destroy_on(&base).await.unwrap();
    let leaked = next_event(&mut stream).await;
    println!("after destroying another owner's record: {leaked:?}");
    assert!(
        leaked.is_none(),
        "viewer heard the destroy of a record it cannot read: {leaked:?}"
    );

    mine.destroy_on(&base).await.unwrap();
    let own = next_event(&mut stream).await;
    println!("after destroying the viewer's own record: {own:?}");
    assert_eq!(
        own.unwrap()["ownedDestroyed"]["destroyed"],
        json!(mine.id.to_string())
    );
}

/// Destroy predicates are evaluated before projection.
#[tokio::test]
async fn destroy_event_skips_subscribers_whose_filter_excludes_it() {
    let pubsub = PubSub::new();
    let runtime = ash_core::Runtime::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let base = runtime.bootstrap();
    let schema = AshGraphQL::from_resources(&[&Owned::DEF])
        .with_pubsub(pubsub)
        .finish::<Memory>()
        .unwrap();
    let viewer = Actor::new(Uuid::new_v4());
    let ctx = runtime.bind(Some(viewer.clone()), None);

    let open = Owned::create(&base)
        .title("open one")
        .owner_id(viewer.id)
        .status("open")
        .await
        .unwrap();
    let closed = Owned::create(&base)
        .title("closed one")
        .owner_id(viewer.id)
        .status("closed")
        .await
        .unwrap();

    let mut stream = schema.execute_stream(
        Request::new(r#"subscription { ownedDestroyed(filter: { status: { eq: "closed" } }) { destroyed } }"#).data(ctx.clone()),
    );
    assert!(next_event(&mut stream).await.is_none());
    settle().await;

    open.destroy_on(&base).await.unwrap();
    let excluded = next_event(&mut stream).await;
    println!("after destroying a record the filter excludes: {excluded:?}");
    assert!(
        excluded.is_none(),
        "filter excluded the record but the destroy was delivered: {excluded:?}"
    );

    closed.destroy_on(&base).await.unwrap();
    let matched = next_event(&mut stream).await;
    println!("after destroying a record the filter matches: {matched:?}");
    assert_eq!(
        matched.unwrap()["ownedDestroyed"]["destroyed"],
        json!(closed.id.to_string())
    );
}

#[tokio::test]
async fn relationship_snapshot_filters_fail_at_registration_for_create_and_destroy() {
    let pubsub = PubSub::new();
    let runtime = ash_core::Runtime::new(Memory::new());
    let schema = AshGraphQL::from_resources(&[&Team::DEF, &Task::DEF])
        .with_pubsub(pubsub)
        .finish::<Memory>()
        .unwrap();
    for query in [
        r#"subscription { taskCreated(filter: { team: { name: { eq: "red" } } }) { created { title } } }"#,
        r#"subscription { taskDestroyed(filter: { team: { name: { eq: "red" } } }) { destroyed } }"#,
    ] {
        let mut stream = schema.execute_stream(Request::new(query).data(runtime.bootstrap()));
        let response = stream.next().await.unwrap();
        assert!(
            response
                .errors
                .iter()
                .any(|error| error.message.contains("relationships")),
            "{:?}",
            response.errors
        );
        assert!(stream.next().await.is_none());
    }
}

/// An event from another tenant arriving first does not drop a later event for
/// the subscriber's tenant, and no event crosses tenants (attribute strategy).
#[tokio::test]
async fn attribute_tenant_events_stay_in_tenant_and_are_not_dropped() {
    let pubsub = PubSub::new();
    let runtime = ash_core::Runtime::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let (a, b) = (
        runtime.bind(None, Some("a".into())),
        runtime.bind(None, Some("b".into())),
    );
    let schema = AshGraphQL::from_resources(&[&Shelf::DEF])
        .with_pubsub(pubsub)
        .finish::<Memory>()
        .unwrap();

    let mut stream = schema.execute_stream(
        Request::new("subscription { shelfCreated { created { label org } } }").data(a.clone()),
    );
    assert!(next_event(&mut stream).await.is_none());
    settle().await;

    // Back-to-back: b's first, then a's.
    Shelf::create(&b).label("b first").await.unwrap();
    Shelf::create(&a).label("a second").await.unwrap();
    Shelf::create(&b).label("b third").await.unwrap();

    let first = next_event(&mut stream).await;
    let second = next_event(&mut stream).await;
    println!("tenant a heard: {first:?} then {second:?}");
    assert_eq!(
        first.unwrap()["shelfCreated"]["created"]["label"],
        json!("a second")
    );
    assert!(
        second.is_none(),
        "tenant a heard another tenant's event: {second:?}"
    );

    // Control: tenant a's list query.
    let listed = schema
        .execute(Request::new("{ listShelfs { label } }").data(a.clone()))
        .await;
    assert!(listed.errors.is_empty(), "{:?}", listed.errors);
    let listed = listed.data.into_json().unwrap();
    println!("control list in tenant a: {listed}");
    assert_eq!(listed["listShelfs"], json!([{ "label": "a second" }]));
}

/// Context tenancy uses the bound scope, since snapshots have no tenant column.
#[tokio::test]
async fn context_tenant_events_stay_in_tenant_and_are_not_dropped() {
    let pubsub = PubSub::new();
    let runtime = ash_core::Runtime::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let base = runtime.bootstrap();
    let (a, b) = (
        runtime.bind(None, Some("a".into())),
        runtime.bind(None, Some("b".into())),
    );
    let schema = AshGraphQL::from_resources(&[&Vault::DEF])
        .with_pubsub(pubsub)
        .finish::<Memory>()
        .unwrap();

    let mut stream = schema.execute_stream(
        Request::new("subscription { vaultCreated { created { label } } }").data(a.clone()),
    );
    assert!(next_event(&mut stream).await.is_none());
    settle().await;

    Vault::create(&b).label("b first").await.unwrap();
    Vault::create(&a).label("a second").await.unwrap();
    Vault::create(&b).label("b third").await.unwrap();

    let first = next_event(&mut stream).await;
    let second = next_event(&mut stream).await;
    println!("tenant a heard: {first:?} then {second:?}");
    assert_eq!(
        first.unwrap()["vaultCreated"]["created"]["label"],
        json!("a second")
    );
    assert!(
        second.is_none(),
        "tenant a heard another tenant's event: {second:?}"
    );

    // A required tenant must be bound before registration.
    let mut untenanted = schema.execute_stream(
        Request::new("subscription { vaultCreated { created { label } } }").data(base),
    );
    let rejected = untenanted.next().await.unwrap();
    assert!(
        rejected
            .errors
            .iter()
            .any(|error| error.message.contains("tenant"))
    );
    assert!(untenanted.next().await.is_none());
}
