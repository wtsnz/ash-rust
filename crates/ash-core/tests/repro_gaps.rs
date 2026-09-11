//! Tests for gaps found in review. Each test states the behavior the engine
//! should have. They fail on the current code.

use std::sync::{Arc, Mutex};

use ash_core::{
    ActionKind, BulkDestroyOptions, Context, CustomValidation, Error, FieldMap, Notification,
    Resource, Result, SyncFnNotifier, ValidationContext, Value, create_dynamic, resource,
};
use ash_memory::Memory;
use uuid::Uuid;

mod task {
    use super::*;

    resource! {
        Task {
        table "repro_tasks";

        attributes {
            id: Uuid [pk];
            title: String;
            status: Option<String>;
        }

        actions {
            create create {
                primary;
                accept [title];
                change set_attribute(status, "open");
            }

            read read {
                primary;
            }
        }
    }}
}

mod draft {
    use super::*;

    resource! {
        Draft {
        table "repro_drafts";

        attributes {
            id: Uuid [pk];
            title: String;
        }

        actions {
            create create {
                primary;
                accept [title];
                validate present(title);
                before_action blank_title;
            }

            read read {
                primary;
            }
        }
    }}

    fn blank_title(fields: &mut FieldMap) -> Result<()> {
        fields.insert("title".into(), Value::String(String::new()));
        Ok(())
    }
}

mod vault {
    use super::*;

    resource! {
        VaultItem {
        table "repro_vault";

        attributes {
            id: Uuid [pk];
            name: String;
            locked: bool;
        }

        actions {
            create create {
                primary;
                accept [name, locked];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
                validate custom(&RejectLocked);
            }
        }
    }}

    pub struct RejectLocked;
    impl CustomValidation for RejectLocked {
        fn validate(&self, ctx: &ValidationContext<'_>) -> Result<()> {
            if matches!(ctx.fields.get("locked"), Some(Value::Bool(true))) {
                return Err(Error::Validation {
                    field: "locked".into(),
                    message: "cannot destroy a locked record".into(),
                });
            }
            Ok(())
        }
    }
}

mod tenant_note {
    use super::*;

    resource! {
        TenantNote {
        table "repro_tenant_notes";

        multitenancy {
            strategy: attribute;
            attribute: "tenant_id";
        }

        attributes {
            id: Uuid [pk];
            tenant_id: String;
            title: String;
        }

        actions {
            create create {
                primary;
                accept [title];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

mod notice {
    use super::*;

    resource! {
        Notice {
        table "repro_notices";

        attributes {
            id: Uuid [pk];
            body: String;
        }

        actions {
            create create {
                primary;
                accept [body];
            }

            read read {
                primary;
            }
        }
    }}
}

use notice::Notice;
use task::Task;
use tenant_note::TenantNote;
use vault::VaultItem;

#[tokio::test]
async fn create_dynamic_runs_action_changes() {
    let ctx = Context::new(Memory::new());

    let via_builder = Task::create(&ctx)
        .title("typed path")
        .await
        .expect("typed create");
    assert_eq!(via_builder.status.as_deref(), Some("open"));

    let action = Task::DEF
        .actions
        .iter()
        .find(|a| a.kind == ActionKind::Create)
        .expect("create action");

    let mut input = FieldMap::new();
    input.insert("title".into(), Value::String("dynamic path".into()));

    let stored = create_dynamic(&ctx, &Task::DEF, action, input)
        .await
        .expect("create_dynamic should apply change set_attribute(status, open)");

    assert_eq!(
        stored.get("status").and_then(|v| v.as_str()),
        Some("open"),
        "create_dynamic skipped apply_changes; GraphQL and managed relationships use this path"
    );
}

#[tokio::test]
async fn before_action_is_revalidated_before_persist() {
    let ctx = Context::new(Memory::new());

    let err = draft::Draft::create(&ctx)
        .title("valid at construction")
        .await
        .expect_err("before_action blanked title; persist must re-run present(title)");

    assert!(
        matches!(err, Error::Validation { .. }),
        "expected Validation, got {err:?}"
    );
}

#[tokio::test]
async fn destroy_by_id_runs_action_validations() {
    let ctx = Context::new(Memory::new());

    let item = VaultItem::create(&ctx)
        .name("secrets")
        .locked(true)
        .await
        .unwrap();

    let via_record = item.destroy_on(&ctx).await;
    assert!(
        via_record.is_err(),
        "destroy_on goes through Changeset and must reject locked records"
    );

    let via_id = VaultItem::destroy(&ctx, item.id).await;
    assert!(
        via_id.is_err(),
        "destroy(id) uses destroy_dynamic, which never calls run_validations"
    );
}

#[tokio::test]
async fn count_is_scoped_to_tenant() {
    let mem = Memory::new();
    let alpha = Context::new(mem.clone()).with_tenant("org_alpha");
    let beta = Context::new(mem).with_tenant("org_beta");

    TenantNote::create(&alpha).title("alpha-1").await.unwrap();
    TenantNote::create(&alpha).title("alpha-2").await.unwrap();
    TenantNote::create(&beta).title("beta-1").await.unwrap();

    let listed = TenantNote::query(&alpha).all().await.unwrap();
    assert_eq!(listed.len(), 2);

    let counted = TenantNote::query(&alpha)
        .count()
        .await
        .expect("count should apply the same tenant filter as all()");
    assert_eq!(
        counted, 2,
        "count() skipped the tenant attribute filter; it counted every tenant"
    );
}

#[tokio::test]
async fn bulk_destroy_does_not_delete_other_tenants_by_id() {
    let mem = Memory::new();
    let alpha = Context::new(mem.clone()).with_tenant("org_alpha");
    let beta = Context::new(mem).with_tenant("org_beta");

    let foreign = TenantNote::create(&beta)
        .title("beta-secret")
        .await
        .unwrap();

    let result = ash_core::bulk_destroy::<TenantNote, _>(
        &alpha,
        "destroy",
        &[foreign.id],
        BulkDestroyOptions::default(),
    )
    .await;

    let still_there = TenantNote::query(&beta)
        .filter(TenantNote::id.eq(foreign.id))
        .one()
        .await;

    assert!(
        still_there.is_ok(),
        "bulk_destroy loaded the row by PK only; tenant A deleted tenant B's note. result={result:?}"
    );
}

#[tokio::test]
async fn context_transaction_does_not_notify_on_rollback() {
    let received = Arc::new(Mutex::new(Vec::<Notification>::new()));
    let received_clone = Arc::clone(&received);
    let notifier = Arc::new(SyncFnNotifier::new("repro", move |notif| {
        received_clone.lock().unwrap().push(notif.clone());
        Ok(())
    }));

    let ctx = Context::new(Memory::new()).with_notifier(notifier);

    let failed = ctx
        .transaction(|tx| async move {
            Notice::create(&tx).body("should roll back").await?;
            Err::<(), _>(Error::Invalid("forced rollback".into()))
        })
        .await;

    assert!(failed.is_err());

    let list = received.lock().unwrap();
    assert_eq!(
        list.len(),
        0,
        "Context::transaction dispatched the create notification, then rolled back the row. Multi buffers; this path does not."
    );
}
