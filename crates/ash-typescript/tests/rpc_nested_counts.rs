//! A nested page answers as its read does: ordered by the read's prepared sort, with the
//! primary key only breaking ties, and a count of the rows the page itself would show.

use ash_core::{Actor, Context, Value};
use ash_memory::Memory;
use ash_typescript::rpc::Rpc;
use serde_json::{Value as Json, json};
use uuid::Uuid;

mod shelf {
    use ash_core::resource;
    use uuid::Uuid;

    use super::item::Item;
    use super::secret::Secret;

    resource! {
        Shelf {
            table "shelves";

            attributes {
                id: Uuid [pk];
                name: String;
            }

            relationships {
                has_many items: Item [fk: shelf_id];
                has_many secrets: Secret [fk: shelf_id];
            }

            actions {
                read read { primary; }
                create create { primary; accept [name]; }
            }
        }
    }
}

mod item {
    use ash_core::resource;
    use uuid::Uuid;

    use super::shelf::Shelf;

    resource! {
        Item {
            table "items";

            attributes {
                id: Uuid [pk];
                shelf_id: Uuid;
                score: i64;
            }

            relationships {
                belongs_to shelf: Shelf [fk: shelf_id];
            }

            calculations {
                boosted(by: i64): i64 = score + arg(by);
            }

            actions {
                read read {
                    primary;
                    pagination keyset: true, offset: true, countable: true, required: false;
                    prepare sort(score, desc);
                }
                create create { primary; accept [shelf_id, score]; }
            }
        }
    }
}

mod secret {
    use ash_core::resource;
    use uuid::Uuid;

    use super::shelf::Shelf;

    resource! {
        Secret {
            table "secrets";

            attributes {
                id: Uuid [pk];
                shelf_id: Uuid;
                body: String;
            }

            relationships {
                belongs_to shelf: Shelf [fk: shelf_id];
            }

            actor {
                role: String;
            }

            // Only an editor sees which shelf a secret is on.
            field_policies {
                field shelf_id {
                    authorize_if actor_attribute_equals(role, "editor");
                }
            }

            actions {
                read read {
                    primary;
                    pagination keyset: true, countable: true, required: false;
                }
                create create { primary; accept [shelf_id, body]; }
            }
        }
    }
}

use item::Item;
use secret::Secret;
use shelf::Shelf;

struct Library {
    ctx: Context<Memory>,
    rpc: Rpc<Memory>,
    shelf: Uuid,
}

impl Library {
    /// A shelf with items scored 1 to 3 and one secret.
    async fn new() -> Self {
        let ctx = Context::new(Memory::new());
        let shelf = Shelf::create(&ctx).name("Top").await.unwrap().id;
        for score in 1..=3 {
            Item::create(&ctx).shelf_id(shelf).score(score).await.unwrap();
        }
        Secret::create(&ctx).shelf_id(shelf).body("hidden").await.unwrap();
        Self { ctx, rpc: Rpc::new().get_by::<Shelf>("get_shelf", "read", &["id"]), shelf }
    }

    async fn nested(&self, role: &str, relationship: Json) -> Json {
        let actor = Actor::new(Uuid::new_v4()).with_attr("role", Value::from(role));
        let request = json!({ "action": "get_shelf", "getBy": { "id": self.shelf }, "fields": ["id", relationship] });
        let response = self.rpc.run(&self.ctx.with_actor(actor), &request).await;
        assert_eq!(response["success"], true, "{response}");
        response["data"].clone()
    }
}

#[tokio::test]
async fn a_keyset_page_keeps_the_reads_prepared_sort() {
    let library = Library::new().await;
    let first = library.nested("reader", json!({ "items": { "fields": ["score"], "page": { "limit": 1 } } })).await;
    let items = &first["items"];
    assert_eq!(items["type"], "keyset");
    assert_eq!(items["results"], json!([{ "score": 3 }]), "the highest score first, as the read sorts");
    let next = library.nested("reader", json!({ "items": { "fields": ["score"], "page": { "limit": 1, "after": items["nextPage"] } } })).await;
    assert_eq!(next["items"]["results"], json!([{ "score": 2 }]));
}

#[tokio::test]
async fn a_count_filters_on_a_calculation_with_its_arguments() {
    let library = Library::new().await;
    let data = library
        .nested(
            "reader",
            json!({ "items": {
                "fields": ["score", { "boosted": { "args": { "by": 10 } } }],
                "filter": { "boosted": { "greaterThan": 11 } },
                "page": { "limit": 5, "count": true },
            } }),
        )
        .await;
    let items = &data["items"];
    assert_eq!(items["results"].as_array().unwrap().len(), 2, "{items}");
    assert_eq!(items["count"], 2, "the count filters as the page does");
}

#[tokio::test]
async fn a_count_leaves_out_rows_a_field_policy_unlinks() {
    let library = Library::new().await;
    let secrets = json!({ "secrets": { "fields": ["body"], "page": { "limit": 5, "count": true } } });
    let reader = library.nested("reader", secrets.clone()).await;
    assert_eq!(reader["secrets"]["results"], json!([]), "a reader can't see which shelf the secret is on");
    assert_eq!(reader["secrets"]["count"], 0, "so the count doesn't tell them");
    let editor = library.nested("editor", secrets).await;
    assert_eq!(editor["secrets"]["results"], json!([{ "body": "hidden" }]));
    assert_eq!(editor["secrets"]["count"], 1);
}
