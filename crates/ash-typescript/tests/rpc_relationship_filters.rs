//! A relationship in a client's filter reaches only the related rows the actor may read,
//! as Ash authorizes each relationship path in a filter it's given.

use ash_core::{Context, resource};
use ash_memory::Memory;
use ash_typescript::rpc::Rpc;
use serde_json::json;
use uuid::Uuid;

pub mod parent {
    use super::*;

    resource! {
        Parent {
            table "rpc_parents";

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
            table "rpc_children";

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
use parent::Parent;

#[tokio::test]
async fn a_filter_reaches_only_the_related_rows_the_actor_may_read() {
    let ctx = Context::new(Memory::new());
    let parent = Parent::create(&ctx).name("a").await.unwrap();
    Child::create(&ctx).parent_id(parent.id).body("secret").internal(true).await.unwrap();
    Child::create(&ctx).parent_id(parent.id).body("hello").internal(false).await.unwrap();
    let rpc = Rpc::new().action::<Parent>("list_parents", "read");
    let list = |body: &str| json!({ "action": "list_parents", "fields": ["name"], "filter": { "children": { "body": { "eq": body } } } });

    let hidden = rpc.run(&ctx, &list("secret")).await;
    assert_eq!(hidden, json!({ "success": true, "data": [] }), "found by a child it can't read");
    let visible = rpc.run(&ctx, &list("hello")).await;
    assert_eq!(visible, json!({ "success": true, "data": [{ "name": "a" }] }));
}
