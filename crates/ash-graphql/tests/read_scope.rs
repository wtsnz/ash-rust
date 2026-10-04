//! Every GraphQL read runs as the request's context, through the same scoping as typed
//! reads: connections see the read action's preparations, relationships load through
//! the destination's policies, filters and tenant (batched per request), and field
//! policies redact for the request's actor.

use ash_core::{Actor, Context, Resource};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;
use serde_json::{Value, json};
use uuid::Uuid;

use berth::Berth;
use dock::Dock;

mod dock {
use super::berth::Berth;
use ash_core::resource;
use uuid::Uuid;

resource! {
    Dock {
        table "docks";

        multitenancy {
            strategy: attribute;
            attribute: org;
        }

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            org: String;
            name: String;
            notes: Option<String>;
        }

        relationships {
            has_many berths: Berth [fk: dock_id];
        }

        field_policies {
            field notes {
                authorize_if actor_eq(role = "harbourmaster");
            }
        }

        actions {
            create create { primary; accept [name, notes]; }
            read read { primary; pagination keyset: true, countable: true, required: false; }
        }

        policies {
            policy action_type(create) {
                authorize_if always;
            }
            policy action_type(read) {
                authorize_if actor_present;
            }
        }
    }
}

}

mod berth {
use super::dock::Dock;
use ash_core::resource;
use uuid::Uuid;

resource! {
    Berth {
        table "berths";

        multitenancy {
            strategy: attribute;
            attribute: org;
        }

        attributes {
            id: Uuid [pk];
            org: String;
            dock_id: Uuid;
            code: String;
            closed: bool [default: false];
        }

        relationships {
            belongs_to dock: Dock [fk: dock_id];
        }

        actions {
            create create { primary; accept [dock_id, code, closed]; }
            read read {
                primary;
                pagination keyset: true, countable: true, required: false;
                prepare filter(closed == false);
            }
        }
    }
}

}

struct Harbour {
    schema: async_graphql::dynamic::Schema,
    acme: Context<Memory>,
    globex: Context<Memory>,
}

impl Harbour {
    async fn new() -> Self {
        let base = Context::new(Memory::new());
        let acme = base.with_tenant("acme");
        let globex = base.with_tenant("globex");
        // Field policies guard writes too, so the harbourmaster writes the notes.
        let north = Dock::create(&acme.with_actor(role("harbourmaster")))
            .name("North").notes("keys under the mat").await.unwrap();
        Berth::create(&acme).dock_id(north.id).code("A").await.unwrap();
        Berth::create(&acme).dock_id(north.id).code("B").closed(true).await.unwrap();
        // A berth in another tenant naming acme's dock doesn't reach it.
        Berth::create(&globex).dock_id(north.id).code("C").await.unwrap();

        let schema = AshGraphQL::from_resources(&[&Dock::DEF, &Berth::DEF])
            .with_dataloader()
            .finish::<Memory>()
            .unwrap();
        Self { schema, acme, globex }
    }

    async fn run(&self, request: Request) -> Value {
        let response = self.schema.execute(request).await;
        assert!(response.errors.is_empty(), "{:?}", response.errors);
        response.data.into_json().unwrap()
    }

    async fn as_(&self, ctx: &Context<Memory>, query: &str) -> Value {
        self.run(Request::new(query).data(ctx.clone())).await
    }
}

fn role(role: &str) -> Actor {
    Actor::new(Uuid::new_v4()).with_role(role)
}

const DOCKS: &str = "{ listDocks { results { name notes berths { code dock { name } } } } }";

#[tokio::test]
async fn relationships_load_as_the_request_reads() {
    let h = Harbour::new().await;

    // The harbourmaster sees the notes, and only the open berth.
    let harbourmaster = h.acme.with_actor(role("harbourmaster"));
    assert_eq!(
        h.as_(&harbourmaster, DOCKS).await["listDocks"]["results"],
        json!([{
            "name": "North",
            "notes": "keys under the mat",
            "berths": [{ "code": "A", "dock": { "name": "North" } }],
        }])
    );

    // A clerk reads the same docks without the notes.
    let clerk = h.acme.with_actor(role("clerk"));
    assert_eq!(h.as_(&clerk, DOCKS).await["listDocks"]["results"][0]["notes"], Value::Null);

    // Without an actor the dock's read policy hides it, through a berth too.
    let berths = "{ listBerths { results { code dock { name } } } }";
    assert_eq!(
        h.as_(&h.acme, berths).await["listBerths"]["results"],
        json!([{ "code": "A", "dock": null }])
    );

    // Another tenant's berth can't reach acme's dock.
    let globex = h.globex.with_actor(role("harbourmaster"));
    assert_eq!(
        h.as_(&globex, berths).await["listBerths"]["results"],
        json!([{ "code": "C", "dock": null }])
    );
}

#[tokio::test]
async fn an_actor_given_alongside_the_context_acts_for_it() {
    let h = Harbour::new().await;
    let request = Request::new(DOCKS)
        .data(h.acme.clone())
        .data(role("harbourmaster"));
    let docks = h.run(request).await;
    assert_eq!(docks["listDocks"]["results"][0]["notes"], "keys under the mat");
    assert_eq!(docks["listDocks"]["results"][0]["berths"][0]["dock"]["name"], "North");
}

#[tokio::test]
async fn mutations_run_as_the_actor_given_alongside_the_context() {
    let h = Harbour::new().await;
    // Only the harbourmaster may read a dock's notes; anyone may write them, since field
    // policies govern reads, as in Ash. The mutation's result is read as the actor.
    let mutation = r#"mutation { createDock(input: { name: "South", notes: "mind the gap" }) {
        errors { message } result { name notes }
    } }"#;
    let created = h
        .run(Request::new(mutation).data(h.acme.clone()).data(role("harbourmaster")))
        .await;
    assert_eq!(created["createDock"]["result"]["notes"], "mind the gap", "{created}");
    let clerk = h.run(Request::new(mutation).data(h.acme.clone()).data(role("clerk"))).await;
    assert_eq!(clerk["createDock"]["result"]["name"], "South", "{clerk}");
    assert!(clerk["createDock"]["result"]["notes"].is_null(), "redacted for the clerk: {clerk}");
}

#[tokio::test]
async fn pages_read_through_the_read_action() {
    let h = Harbour::new().await;
    let page = h
        .as_(&h.acme, "{ listBerths(first: 10) { count results { code } } }")
        .await;
    assert_eq!(page["listBerths"], json!({ "count": 1, "results": [{ "code": "A" }] }));
}

// A client's filter can't find records by a field its actor can't read: the field reads
// as null where it's hidden, as AshGraphql reads it, through relationships too.
#[tokio::test]
async fn filters_read_hidden_fields_as_null() {
    let harbour = Harbour::new().await;
    let by_notes = r#"{ listDocks(filter: { notes: { eq: "keys under the mat" } }) { count results { name } } }"#;
    let berths_by_notes =
        r#"{ listBerths(filter: { dock: { notes: { eq: "keys under the mat" } } }) { results { code } } }"#;

    let deckhand = harbour.acme.with_actor(role("deckhand"));
    assert_eq!(harbour.as_(&deckhand, by_notes).await, json!({ "listDocks": { "count": 0, "results": [] } }));
    assert_eq!(harbour.as_(&deckhand, berths_by_notes).await, json!({ "listBerths": { "results": [] } }));
    // Nil wherever it's hidden.
    let unnoted = r#"{ listDocks(filter: { notes: { isNil: true } }) { results { name } } }"#;
    assert_eq!(harbour.as_(&deckhand, unnoted).await, json!({ "listDocks": { "results": [{ "name": "North" }] } }));

    let harbourmaster = harbour.acme.with_actor(role("harbourmaster"));
    assert_eq!(
        harbour.as_(&harbourmaster, by_notes).await,
        json!({ "listDocks": { "count": 1, "results": [{ "name": "North" }] } })
    );
    assert_eq!(harbour.as_(&harbourmaster, berths_by_notes).await, json!({ "listBerths": { "results": [{ "code": "A" }] } }));
}
