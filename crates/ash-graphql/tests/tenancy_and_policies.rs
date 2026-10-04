//! What a subscriber hears and what a client's filter reaches, as AshGraphql decides:
//! no change from another tenant, even of a resource whose tenants live apart (context
//! multitenancy, with no tenant attribute to filter on); each record's fields as the
//! subscriber may read them, not as the writer may; and a relationship in a filter
//! reaches only the related rows the actor may read.

use std::sync::Arc;
use std::time::Duration;

use ash_core::{Actor, Context, Resource, resource};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_pubsub::{ContextPubSubExt, PubSub};
use async_graphql::Request;
use futures_util::StreamExt;
use serde_json::json;
use uuid::Uuid;

pub mod isolated {
    use super::*;

    resource! {
        Isolated {
            table "isolated";

            multitenancy {
                strategy: context;
            }

            attributes {
                id: Uuid [pk];
                text: String;
            }

            actions {
                create create { primary; accept [text]; }
                read read { primary; }
            }
        }
    }
}

pub mod protected {
    use super::*;

    resource! {
        Protected {
            table "protected";

            actor {
                role: String;
            }

            attributes {
                id: Uuid [pk];
                email: Option<String>;
            }

            field_policies {
                field email { authorize_if actor_attribute_equals(role, "admin"); }
            }

            actions {
                create create { primary; accept [email]; }
                read read { primary; }
            }
        }
    }
}

pub mod parent {
    use super::*;

    resource! {
        Parent {
            table "parents";

            attributes {
                id: Uuid [pk];
                name: String;
            }

            relationships {
                has_many children: super::child::Child [fk: parent_id];
            }

            actions {
                create create { primary; accept [name]; }
                read read { primary; }
            }
        }
    }
}

pub mod child {
    use super::*;

    resource! {
        Child {
            table "children";

            attributes {
                id: Uuid [pk];
                parent_id: Uuid;
                body: String;
                internal: bool;
            }

            relationships {
                belongs_to parent: super::parent::Parent [fk: parent_id];
            }

            policies {
                policy action_type(create) { authorize_if always; }
                policy action_type(read) { authorize_if eq(internal, false); }
            }

            actions {
                create create { primary; accept [parent_id, body, internal]; }
                read read { primary; }
            }
        }
    }
}

use child::Child;
use isolated::Isolated;
use parent::Parent;
use protected::Protected;

/// Nothing arrives within the wait.
async fn quiet(stream: &mut (impl futures_util::Stream + Unpin)) -> bool {
    tokio::time::timeout(Duration::from_millis(100), stream.next()).await.is_err()
}

#[tokio::test]
async fn a_subscriber_hears_nothing_from_another_context_tenant() {
    let pubsub = PubSub::new();
    let base = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let (a, b) = (base.with_tenant("a"), base.with_tenant("b"));
    let schema = AshGraphQL::from_resources(&[&Isolated::DEF]).with_pubsub(pubsub).finish::<Memory>().unwrap();
    let mut stream = schema.execute_stream(Request::new("subscription { isolatedCreated { created { text } } }").data(a.clone()));
    assert!(quiet(&mut stream).await);

    Isolated::create(&b).text("tenant b's").await.unwrap();
    assert!(quiet(&mut stream).await, "tenant a heard tenant b's record");

    Isolated::create(&a).text("tenant a's").await.unwrap();
    let event = tokio::time::timeout(Duration::from_millis(500), stream.next()).await.unwrap().unwrap();
    assert_eq!(event.data.into_json().unwrap()["isolatedCreated"]["created"]["text"], json!("tenant a's"));
}

#[tokio::test]
async fn a_subscriber_reads_fields_as_its_own_policies_allow() {
    let pubsub = PubSub::new();
    let base = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let admin = base.with_actor(Actor::new(Uuid::new_v4()).with_attr("role", "admin"));
    let schema = AshGraphQL::from_resources(&[&Protected::DEF]).with_pubsub(pubsub).finish::<Memory>().unwrap();
    let mut stream = schema.execute_stream(Request::new("subscription { protectedCreated { created { email } } }").data(admin));
    assert!(quiet(&mut stream).await);

    // Written by someone who may not read the email; heard by an admin, who may.
    let written = Protected::create(&base).email("s@example.com").await.unwrap();
    assert_eq!(written.email, None, "the writer reads it as its policies allow");
    let event = tokio::time::timeout(Duration::from_millis(500), stream.next()).await.unwrap().unwrap();
    assert!(event.errors.is_empty(), "{:?}", event.errors);
    assert_eq!(event.data.into_json().unwrap()["protectedCreated"]["created"]["email"], json!("s@example.com"));
}

#[tokio::test]
async fn a_filter_reaches_only_the_related_rows_the_actor_may_read() {
    let ctx = Context::new(Memory::new());
    let parent = Parent::create(&ctx).name("a").await.unwrap();
    Child::create(&ctx).parent_id(parent.id).body("secret").internal(true).await.unwrap();
    Child::create(&ctx).parent_id(parent.id).body("hello").internal(false).await.unwrap();
    let schema = AshGraphQL::from_resources(&[&Parent::DEF, &Child::DEF]).finish::<Memory>().unwrap();
    let list = |body: &str| format!(r#"{{ listParents(filter: {{ children: {{ body: {{ eq: "{body}" }} }} }}) {{ name }} }}"#);

    let hidden = schema.execute(Request::new(list("secret")).data(ctx.clone())).await;
    assert!(hidden.errors.is_empty(), "{:?}", hidden.errors);
    assert_eq!(hidden.data.into_json().unwrap()["listParents"], json!([]), "found by a child it can't read");

    let visible = schema.execute(Request::new(list("hello")).data(ctx)).await;
    assert_eq!(visible.data.into_json().unwrap()["listParents"], json!([{ "name": "a" }]));
}
