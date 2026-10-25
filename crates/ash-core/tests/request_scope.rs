use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ash_core::{
    Actor, Changeset, DataLayer, DynamicChangeset, Error, Resource, Runtime, TransactionSupport,
};
use ash_memory::Memory;
use uuid::Uuid;

ash_core::resource! {
    Entry {
        table "request_scope_entries";
        actor { role: String; }
        multitenancy { strategy: attribute; attribute: org; }
        attributes {
            id: Uuid [pk];
            org: String;
            secret: String;
        }
        policies {
            policy always { authorize_if actor_attribute_equals(role, "admin"); }
        }
        field_policies {
            field secret { authorize_if actor_attribute_equals(role, "admin"); }
        }
        actions {
            create create { primary; accept [secret]; }
            read read { primary; }
            update update { primary; accept [secret]; }
            destroy destroy { primary; }
        }
    }
}

fn admin() -> Actor {
    Actor::new(Uuid::new_v4()).with_role("admin")
}

#[tokio::test]
async fn changesets_reject_another_scope_before_hooks_or_writes() {
    let runtime = Runtime::new(Memory::new());
    let actor = admin();
    let prepared = runtime.bind(Some(actor.clone()), Some("a".into()));
    let other_scopes = [
        runtime.bind(Some(actor.clone()), Some("b".into())),
        runtime.bind(Some(Actor::new(Uuid::new_v4())), Some("a".into())),
        runtime.bind(Some(actor), Some("a".into())),
    ];
    for other in &other_scopes {
        let ran = Arc::new(AtomicBool::new(false));
        let hook = Arc::clone(&ran);
        let cs = Entry::create(&prepared)
            .secret("private")
            .before_action(move |_| {
                hook.store(true, Ordering::SeqCst);
                Ok(())
            })
            .changeset()
            .unwrap();
        assert!(
            matches!(cs.commit(other).await, Err(Error::Invalid(message)) if message.contains("different request scope"))
        );
        assert!(!ran.load(Ordering::SeqCst));
        assert!(Entry::query(&prepared).all().await.unwrap().is_empty());
    }
    let existing = Entry::create(&prepared).secret("original").await.unwrap();
    for action in ["update", "destroy"] {
        let cs = if action == "update" {
            Changeset::for_update_on(
                &prepared,
                action,
                existing.clone(),
                ash_core::fields! { "secret" => "changed" },
            )
            .unwrap()
        } else {
            Changeset::for_destroy(&prepared, action, existing.clone()).unwrap()
        };
        assert!(matches!(
            cs.commit(&other_scopes[2]).await,
            Err(Error::Invalid(_))
        ));
    }
    assert_eq!(
        Entry::get(&prepared, existing.id).await.unwrap().secret,
        "original"
    );
}

#[tokio::test]
async fn dynamic_changesets_reject_another_scope() {
    let runtime = Runtime::new(Memory::new());
    let a = runtime.system().bind(None, Some("a".into()));
    let b = runtime.system().bind(None, Some("b".into()));
    let cs = DynamicChangeset::for_create(
        &a,
        &Entry::DEF,
        Entry::DEF.action("create").unwrap(),
        ash_core::fields! { "secret" => "private" },
    )
    .unwrap();
    assert!(matches!(cs.commit(&b).await, Err(Error::Invalid(_))));
    assert!(Entry::query(&a).all().await.unwrap().is_empty());
    assert!(Entry::query(&b).all().await.unwrap().is_empty());
}

async fn transaction_keeps_scope<D: DataLayer + TransactionSupport>(runtime: Runtime<D>) {
    let ctx = runtime.bind(Some(admin()), Some("a".into()));
    let original = ctx.clone();
    let derived = ctx.with_metadata("trace", "same-request");
    assert!(ctx.same_scope(&derived));
    let (buffered, _) = ctx.with_notification_buffer();
    assert!(ctx.same_scope(&buffered));
    let cs = Entry::create(&ctx).secret("committed").changeset().unwrap();
    ctx.transaction(move |tx| async move {
        assert!(original.same_scope(&tx));
        assert_eq!(original.actor(), tx.actor());
        assert_eq!(original.tenant(), tx.tenant());
        let clone = tx.clone();
        assert!(clone.same_scope(&tx));
        cs.commit(&clone).await?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(Entry::query(&ctx).one().await.unwrap().secret, "committed");
}

#[tokio::test]
async fn memory_transactions_keep_the_exact_scope() {
    transaction_keeps_scope(Runtime::new(Memory::new())).await;
}

#[tokio::test]
async fn sqlite_transactions_keep_the_exact_scope() {
    let db = ash_sqlite::Sqlite::memory().await.unwrap();
    db.install(&[&Entry::DEF]).await.unwrap();
    transaction_keeps_scope(Runtime::new(db)).await;
}

#[tokio::test]
async fn system_scopes_bypass_authorization_and_keep_required_tenancy() {
    let runtime = Runtime::new(Memory::new());
    assert!(matches!(
        Entry::create(&runtime.system().bind(None, None))
            .secret("no tenant")
            .await,
        Err(Error::TenantRequired { .. })
    ));
    for tenant in ["a", "b"] {
        let system = runtime.system().bind(None, Some(tenant.into()));
        let record = Entry::create(&system).secret(tenant).await.unwrap();
        assert_eq!(record.org, tenant);
        let record = record.update_on(&system).secret("updated").await.unwrap();
        assert_eq!(record.secret, "updated");
        assert_eq!(Entry::query(&system).one().await.unwrap().secret, "updated");
        record.destroy_on(&system).await.unwrap();
        let ordinary = runtime.bind(
            Some(Actor::new(Uuid::nil()).with_role("system")),
            Some(tenant.into()),
        );
        assert!(matches!(
            Entry::create(&ordinary).secret("forged").await,
            Err(Error::Forbidden)
        ));
    }
}

async fn runtime_transactions_keep_scopes_separate<D: TransactionSupport>(runtime: Runtime<D>) {
    let sent = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&sent);
    let notifier = Arc::new(ash_core::SyncFnNotifier::new(
        "scope-transactions",
        move |_: &ash_core::Notification| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    ));
    let grant = runtime.notifier_grant::<Entry>(&[]).unwrap();
    let runtime = runtime.with_notifier(grant.attach(notifier));
    let buffered = Arc::clone(&sent);
    let result: ash_core::Result<()> = runtime
        .transaction(move |tx| async move {
            let a = tx.system().bind(None, Some("a".into()));
            let b = tx.system().bind(None, Some("b".into()));
            assert!(!a.same_scope(&b));
            Entry::create(&a).secret("a").await?;
            Entry::create(&b).secret("b").await?;
            assert_eq!(Entry::query(&a).one().await?.org, "a");
            assert_eq!(Entry::query(&b).one().await?.org, "b");
            assert_eq!(buffered.load(Ordering::SeqCst), 0);
            Err(Error::Invalid("abort".into()))
        })
        .await;
    assert!(result.is_err());
    assert_eq!(sent.load(Ordering::SeqCst), 0);
    for tenant in ["a", "b"] {
        let ctx = runtime.system().bind(None, Some(tenant.into()));
        assert!(Entry::query(&ctx).all().await.unwrap().is_empty());
    }
    let buffered = Arc::clone(&sent);
    // System authority also survives the Runtime transaction boundary.
    runtime
        .system()
        .transaction(move |tx| async move {
            for tenant in ["a", "b"] {
                let ctx = tx.bind(None, Some(tenant.into()));
                Entry::create(&ctx).secret(tenant).await?;
            }
            assert_eq!(buffered.load(Ordering::SeqCst), 0);
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(sent.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn memory_runtime_transactions_keep_scopes_separate_and_roll_back_together() {
    runtime_transactions_keep_scopes_separate(Runtime::new(Memory::new())).await;
}

#[tokio::test]
async fn sqlite_runtime_transactions_keep_scopes_separate_and_roll_back_together() {
    let db = ash_sqlite::Sqlite::memory().await.unwrap();
    db.install(&[&Entry::DEF]).await.unwrap();
    runtime_transactions_keep_scopes_separate(Runtime::new(db)).await;
}

struct RequireNoActor;
static NO_ACTOR: RequireNoActor = RequireNoActor;

impl ash_core::CustomValidation for RequireNoActor {
    fn validate(&self, ctx: &ash_core::ValidationContext<'_>) -> ash_core::Result<()> {
        assert!(
            ctx.actor.is_none(),
            "system authorization must not invent an actor"
        );
        Ok(())
    }

    fn atomic(&self, ctx: &ash_core::AtomicContext<'_>) -> ash_core::Atomic {
        assert!(
            ctx.actor.is_none(),
            "atomic validation must see the bound actor"
        );
        ash_core::Atomic::nothing()
    }
}

mod probe {
    use super::*;

    ash_core::resource! {
        ActorProbe {
            table "request_scope_actor_probes";
            actor { role: String; }
            attributes {
                id: Uuid [pk];
                label: String;
                owner: Option<Uuid>;
            }
            policies { policy always { authorize_if actor_attribute_equals(role, "admin"); } }
            actions {
                create create { primary; accept [label]; validate custom(&NO_ACTOR); }
                read read { primary; }
                update update { primary; accept [label]; validate custom(&NO_ACTOR); }
                update claim { accept [label]; change relate_actor(owner); }
            }
        }
    }
}

use probe::ActorProbe;

async fn system_authority_preserves_identity<D: TransactionSupport>(runtime: Runtime<D>) {
    let system = runtime.system().bind(None, None);
    assert!(system.actor().is_none());
    let record = ActorProbe::create(&system).label("created").await.unwrap();
    let record = ActorProbe::update(&system, record.id)
        .label("updated")
        .await
        .unwrap();
    assert_eq!(record.label, "updated");
    assert!(matches!(
        ActorProbe::claim(&system, record.id)
            .label("missing actor")
            .await,
        Err(Error::Forbidden)
    ));
    let actor = Actor::new(Uuid::new_v4()).with_role("worker");
    let as_actor = runtime.system().bind(Some(actor.clone()), None);
    assert_eq!(as_actor.actor(), Some(&actor));
    let claimed = ActorProbe::claim(&as_actor, record.id)
        .label("claimed")
        .await
        .unwrap();
    assert_eq!(claimed.owner, Some(actor.id));
    // Rebinding a compiler policy identity does not carry system authority with it.
    let ordinary = runtime.bind(system.authorization_actor().cloned(), Some("a".into()));
    assert!(matches!(
        Entry::create(&ordinary).secret("forged").await,
        Err(Error::Forbidden)
    ));
}

#[tokio::test]
async fn memory_system_authority_preserves_actor_in_changes_and_atomic_validations() {
    system_authority_preserves_identity(Runtime::new(Memory::new())).await;
}

#[tokio::test]
async fn sqlite_system_authority_preserves_actor_in_changes_and_atomic_validations() {
    let db = ash_sqlite::Sqlite::memory().await.unwrap();
    db.install(&[&Entry::DEF, &ActorProbe::DEF]).await.unwrap();
    system_authority_preserves_identity(Runtime::new(db)).await;
}
