//! Typed changesets, the dynamic functions GraphQL calls, and bulk actions all write
//! through one pipeline, so they accept, authorize, validate and notify alike.

use std::sync::{Arc, Mutex};

use ash_core::{
    Actor, BulkCreateOptions, BulkDestroyOptions, Context, Error, FieldMap, Notification,
    Resource, Result, SyncFnNotifier, ValidationContext, Value, resource,
};
use ash_memory::Memory;
use uuid::Uuid;

fn not_locked(ctx: &ValidationContext<'_>) -> Result<()> {
    match ctx.fields.get("locked") {
        Some(Value::Bool(true)) => Err(Error::Validation {
            field: "locked".into(),
            message: "is locked".into(),
        }),
        _ => Ok(()),
    }
}

resource! {
    Note {
        table "notes";

        attributes {
            id: Uuid [pk];
            owner_id: Uuid;
            body: String;
            locked: bool [default: false];
        }

        actions {
            create write {
                primary;
                accept [owner_id, body, locked];
                argument reason: String;
                validate present(reason);
            }
            update edit {
                primary;
                accept [owner_id, body];
            }
            read read { primary; }
            destroy remove {
                primary;
                validate func(not_locked);
            }
        }

        policies {
            policy action_type(read) | action_type(create) {
                authorize_if always;
            }
            policy action_type(update) | action_type(destroy) {
                authorize_if relates_to_actor(owner_id);
            }
        }
    }
}

fn note(owner: Uuid, body: &str) -> FieldMap {
    let mut input = FieldMap::new();
    input.insert("owner_id".into(), Value::Uuid(owner));
    input.insert("body".into(), Value::String(body.into()));
    input.insert("reason".into(), Value::String("filing".into()));
    input
}

fn action(name: &str) -> &'static ash_core::ActionDef {
    Note::DEF.action(name).unwrap()
}

/// Notifications a context sends, kept for inspection.
fn recording() -> (Context<Memory>, Arc<Mutex<Vec<Notification>>>) {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&sent);
    let notifier = SyncFnNotifier::new("log", move |notification: &Notification| {
        log.lock().unwrap().push(notification.clone());
        Ok(())
    });
    (Context::new(Memory::new()).with_notifier(Arc::new(notifier)), sent)
}

#[tokio::test]
async fn every_write_path_notifies_with_its_arguments() {
    let (ctx, sent) = recording();
    let owner = Uuid::new_v4();
    Note::write(&ctx).owner_id(owner).body("typed").reason("filing").await.unwrap();
    ash_core::create_dynamic(&ctx, &Note::DEF, action("write"), note(owner, "dynamic")).await.unwrap();
    ash_core::bulk_create::<Note, _, _, _>(&ctx, "write", [note(owner, "bulk")], BulkCreateOptions::default())
        .await
        .unwrap();

    let sent = sent.lock().unwrap();
    let reasons: Vec<_> = sent.iter().map(|n| n.metadata.get("reason").cloned()).collect();
    assert_eq!(reasons, vec![Some(Value::String("filing".into())); 3]);
}

#[tokio::test]
async fn updates_authorize_against_the_record_as_it_was() {
    let ctx = Context::new(Memory::new());
    let owner = Uuid::new_v4();
    let heir = Uuid::new_v4();
    let as_owner = ctx.with_actor(Actor::new(owner));

    // The owner may hand a note on, through either path: the policy reads the record
    // being updated, as Ash's does, not the one it would become.
    let typed = Note::write(&ctx).owner_id(owner).body("typed").reason("filing").await.unwrap();
    typed.edit_on(&as_owner).owner_id(heir).await.unwrap();
    let dynamic = Note::write(&ctx).owner_id(owner).body("dynamic").reason("filing").await.unwrap();
    let mut input = FieldMap::new();
    input.insert("owner_id".into(), Value::Uuid(heir));
    ash_core::update_dynamic(&as_owner, &Note::DEF, action("edit"), dynamic.id, input.clone())
        .await
        .unwrap();

    // And someone else may not, whatever they set.
    let stranger = ctx.with_actor(Actor::new(Uuid::new_v4()));
    let err = ash_core::update_dynamic(&stranger, &Note::DEF, action("edit"), dynamic.id, input)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Forbidden), "{err:?}");
}

#[tokio::test]
async fn dynamic_writes_take_only_what_the_action_accepts() {
    let ctx = Context::new(Memory::new());
    let mut input = note(Uuid::new_v4(), "sneaky");
    input.insert("id".into(), Value::Uuid(Uuid::new_v4()));
    let err = ash_core::create_dynamic(&ctx, &Note::DEF, action("write"), input)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotAccepted { ref field, .. } if field == "id"), "{err:?}");
}

#[tokio::test]
async fn bulk_destroys_run_the_destroy_action() {
    let ctx = Context::new(Memory::new());
    let owner = Uuid::new_v4();
    let open = Note::write(&ctx).owner_id(owner).body("open").reason("filing").await.unwrap();
    let locked = Note::write(&ctx).owner_id(owner).body("locked").reason("filing").locked(true).await.unwrap();

    let result = ash_core::bulk_destroy::<Note, _>(
        &ctx.with_actor(Actor::new(owner)),
        "remove",
        &[open.id, locked.id],
        BulkDestroyOptions::default().stop_on_error(false),
    )
    .await
    .unwrap();
    assert_eq!((result.count, result.error_count), (1, 1), "{:?}", result.errors);
    let left: Vec<String> = Note::query(&ctx).all().await.unwrap().into_iter().map(|n| n.body).collect();
    assert_eq!(left, ["locked"]);
}
