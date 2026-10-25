use ash_core::{
    Actor, BulkCreateOptions, BulkTransaction, Error, Resource, Runtime, TransactionSupport,
    fields, resource,
};
use uuid::Uuid;

pub mod scoped_note {
    use super::*;
    resource! {
        ScopedNote {
            table "guarded_upsert_notes";
            multitenancy { strategy: attribute; attribute: org; }
            attributes {
                id: Uuid [pk]; org: String; code: String; text: String;
            }
            identities {
                identity unique_text: [text];
                identity unique_code: [code], all_tenants;
            }
            actions {
                create create { primary; accept [code, text]; }
                read read { primary; }
            }
        }
    }
}
pub use scoped_note::ScopedNote;

pub mod owned_note {
    use super::*;
    resource! {
        OwnedNote {
            table "guarded_upsert_owned_notes";
            multitenancy { strategy: attribute; attribute: org; }
            attributes {
                id: Uuid [pk]; org: String; code: String; text: String; owner: Uuid;
                version: i64 [default: 1];
            }
            identities { identity unique_code: [code], all_tenants; }
            policies {
                policy action_type(read) { authorize_if always; }
                policy action_type(create) { authorize_if relates_to_actor(owner); }
            }
            actions {
                create create { primary; accept [code, text, owner, version]; }
                read read { primary; }
            }
        }
    }
}
pub use owned_note::OwnedNote;

pub const RESOURCES: &[&ash_core::ResourceDef] = &[&ScopedNote::DEF, &OwnedNote::DEF];

fn key() -> String {
    Uuid::new_v4().to_string()
}

fn not_found<T: std::fmt::Debug>(result: ash_core::Result<T>) {
    let error = result.unwrap_err();
    assert!(matches!(error, Error::NotFound), "{error:?}");
    assert_eq!(error.to_string(), "not found");
}

pub async fn global_conflicts_stay_in_tenant<D: TransactionSupport + 'static>(runtime: Runtime<D>) {
    let b = runtime.bind(None, Some(key()));
    for system in [false, true] {
        let a = if system {
            runtime.system().bind(None, Some(key()))
        } else {
            runtime.bind(None, Some(key()))
        };
        for updates in [&["text"][..], &[][..]] {
            let theirs = ScopedNote::create(&b)
                .code(key())
                .text(key())
                .await
                .unwrap();
            not_found(
                ScopedNote::create(&a)
                    .code(theirs.code.clone())
                    .text(key())
                    .upsert_on(ScopedNote::unique_code, updates)
                    .await,
            );
            let stored = ScopedNote::get(&b, theirs.id).await.unwrap();
            assert_eq!(stored.id, theirs.id);
            assert_eq!(stored.org, theirs.org);
            assert_eq!(stored.text, theirs.text);
            assert!(ScopedNote::query(&a).all().await.unwrap().is_empty());
            not_found(ScopedNote::get(&a, theirs.id).await);
        }
    }
}

pub async fn bulk_global_conflicts_stay_in_tenant<D: TransactionSupport + 'static>(
    runtime: Runtime<D>,
) {
    let b = runtime.bind(None, Some(key()));
    for system in [false, true] {
        let a = if system {
            runtime.system().bind(None, Some(key()))
        } else {
            runtime.bind(None, Some(key()))
        };
        for updates in [&["text"][..], &[][..]] {
            let theirs = ScopedNote::create(&b)
                .code(key())
                .text(key())
                .await
                .unwrap();
            let own_code = key();
            let inputs = vec![
                fields! { "code" => theirs.code.clone(), "text" => key() },
                fields! { "code" => own_code.clone(), "text" => key() },
            ];
            // Per-row errors let us assert the rejected result contains no record and
            // the unrelated insert still succeeds on all three backends.
            let result = ash_core::bulk_create::<ScopedNote, _, _, _>(
                &a,
                "create",
                inputs,
                BulkCreateOptions::default()
                    .upsert(ScopedNote::unique_code, updates)
                    .transaction(BulkTransaction::Off)
                    .stop_on_error(false),
            )
            .await
            .unwrap();
            assert_eq!(result.error_count, 1);
            assert_eq!(result.count, 1);
            assert_eq!(result.errors.len(), 1);
            assert_eq!(result.errors, vec!["not found"]);
            assert_eq!(result.records.len(), 1);
            assert_eq!(result.records[0].code, own_code);
            assert_eq!(result.records[0].org, a.tenant().unwrap());
            let stored = ScopedNote::get(&b, theirs.id).await.unwrap();
            assert_eq!(
                (stored.id, stored.org, stored.text),
                (theirs.id, theirs.org, theirs.text)
            );
        }
    }
}

pub async fn same_tenant_conflicts_update_only_selected_columns<D: TransactionSupport + 'static>(
    runtime: Runtime<D>,
) {
    let ctx = runtime.bind(None, Some(key()));
    for identity in [ScopedNote::unique_text, ScopedNote::unique_code] {
        for bulk in [false, true] {
            for updates in [true, false] {
                let before = ScopedNote::create(&ctx)
                    .code(key())
                    .text(key())
                    .await
                    .unwrap();
                let (code, text, changed) = if identity == ScopedNote::unique_code {
                    (before.code.clone(), key(), "text")
                } else {
                    (key(), before.text.clone(), "code")
                };
                let update_fields = if updates { vec![changed] } else { vec![] };
                let after = if bulk {
                    let result = ash_core::bulk_create::<ScopedNote, _, _, _>(
                        &ctx,
                        "create",
                        vec![fields! { "code" => code.clone(), "text" => text.clone() }],
                        BulkCreateOptions::default().upsert(identity, &update_fields),
                    )
                    .await
                    .unwrap();
                    assert_eq!(result.error_count, 0);
                    result.records.into_iter().next().unwrap()
                } else {
                    ScopedNote::create(&ctx)
                        .code(code.clone())
                        .text(text.clone())
                        .upsert_on(identity, &update_fields)
                        .await
                        .unwrap()
                };
                assert_eq!(after.id, before.id);
                assert_eq!(after.org, before.org);
                assert_eq!(after.code, if updates { code } else { before.code });
                assert_eq!(after.text, if updates { text } else { before.text });
                // Reading by the returned key also checks memory's table key agrees.
                let stored = ScopedNote::get(&ctx, after.id).await.unwrap();
                assert_eq!(
                    (stored.id, stored.org, stored.code, stored.text),
                    (after.id, after.org, after.code, after.text)
                );
            }
        }
    }
    // Per-tenant identities retain separate rows for the same identity value.
    let other = runtime.bind(None, Some(key()));
    let shared = key();
    let theirs = ScopedNote::create(&other)
        .code(key())
        .text(shared.clone())
        .await
        .unwrap();
    let mine = ScopedNote::create(&ctx)
        .code(key())
        .text(shared)
        .upsert_on(ScopedNote::unique_text, &["code"])
        .await
        .unwrap();
    assert_ne!(mine.id, theirs.id);
    assert_eq!(mine.org, ctx.tenant().unwrap());
}

pub async fn rejected_conflicts_never_reselect_a_row<D: TransactionSupport + 'static>(
    runtime: Runtime<D>,
) {
    let owner = Uuid::new_v4();
    let tenant = key();
    let b = runtime.bind(Some(Actor::new(owner)), Some(tenant.clone()));
    let a_id = Uuid::new_v4();
    let a = runtime.bind(Some(Actor::new(a_id)), Some(tenant.clone()));
    for updates in [&["text"][..], &[][..]] {
        let theirs = OwnedNote::create(&b)
            .code(key())
            .text("private")
            .owner(owner)
            .await
            .unwrap();
        // Input passes create authorization, but the conflict's stored owner does not.
        // A fallback SELECT by identity would return this row despite zero affected rows.
        not_found(
            OwnedNote::create(&a)
                .code(theirs.code.clone())
                .text("overwrite")
                .owner(a_id)
                .upsert_on(OwnedNote::unique_code, updates)
                .await,
        );
        let bulk = ash_core::bulk_create::<OwnedNote, _, _, _>(
            &a,
            "create",
            vec![fields! { "code" => theirs.code.clone(), "text" => "overwrite", "owner" => a_id }],
            BulkCreateOptions::default()
                .upsert(OwnedNote::unique_code, updates)
                .transaction(BulkTransaction::Off)
                .stop_on_error(false),
        )
        .await
        .unwrap();
        assert!(bulk.records.is_empty());
        assert_eq!(bulk.error_count, 1);
        assert_eq!(bulk.errors, vec!["not found"]);
        let stored = OwnedNote::get(&b, theirs.id).await.unwrap();
        assert_eq!(stored.text, "private");
        assert_eq!(stored.owner, owner);
        // System authority skips write policy within the same tenant.
        let system = runtime.system().bind(None, Some(tenant.clone()));
        let returned = OwnedNote::create(&system)
            .code(theirs.code.clone())
            .text("system")
            .owner(a_id)
            .upsert_on(OwnedNote::unique_code, updates)
            .await
            .unwrap();
        assert_eq!(returned.id, theirs.id);
        assert_eq!(
            returned.text,
            if updates.is_empty() {
                "private"
            } else {
                "system"
            }
        );
        assert_eq!(returned.owner, owner);
    }
}

async fn locked<D: TransactionSupport>(
    ctx: &ash_core::Context<D>,
    code: String,
    text: &str,
    owner: Uuid,
    version: i64,
    updates: &[&str],
) -> ash_core::Result<OwnedNote> {
    // The DSL restricts locks to update/destroy. Handwritten action metadata may carry
    // them on a create, so the executable conflict plan must retain that condition too.
    static ACTION: ash_core::ActionDef = ash_core::ActionDef::create("locked")
        .accept(&["code", "text", "owner", "version"])
        .changes(&[ash_core::Change::OptimisticLock { field: "version" }]);
    let view = ash_core::DynamicChangeset::for_create(
        ctx,
        &OwnedNote::DEF,
        &ACTION,
        fields! { "code" => code, "text" => text, "owner" => owner, "version" => version },
    )?
    .with_upsert(OwnedNote::unique_code, updates)
    .commit(ctx)
    .await?;
    OwnedNote::from_fields(view.fields())
}

pub async fn conflict_locks_check_expected_versions<D: TransactionSupport + 'static>(
    runtime: Runtime<D>,
) {
    let actor = Uuid::new_v4();
    let ctx = runtime.bind(Some(Actor::new(actor)), Some(key()));
    let before = locked(&ctx, key(), "first", actor, 1, &["text"])
        .await
        .unwrap();
    let updated = locked(&ctx, before.code.clone(), "second", actor, 1, &["text"])
        .await
        .unwrap();
    assert_eq!(updated.version, 2);
    assert_eq!(updated.id, before.id);
    for fields in [&["text"][..], &[][..]] {
        not_found(locked(&ctx, before.code.clone(), "stale", actor, 1, fields).await);
    }
    let unchanged = locked(&ctx, before.code, "ignored", actor, 2, &[])
        .await
        .unwrap();
    assert_eq!(unchanged.text, "second");
    assert_eq!(unchanged.version, 2);
}

pub async fn conflict_columns_are_validated_before_writing<D: TransactionSupport + 'static>(
    runtime: Runtime<D>,
) {
    let ctx = runtime.bind(None, Some(key()));
    for field in ["id", "org", "unknown"] {
        let err = ScopedNote::create(&ctx)
            .code(key())
            .text(key())
            .upsert_on(ScopedNote::unique_code, &[field])
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    }
    assert!(ScopedNote::query(&ctx).all().await.unwrap().is_empty());
}

macro_rules! cases {
    ($setup:ident) => {
        #[tokio::test]
        async fn global_conflicts_stay_in_tenant() {
            support::global_conflicts_stay_in_tenant($setup().await).await;
        }
        #[tokio::test]
        async fn bulk_global_conflicts_stay_in_tenant() {
            support::bulk_global_conflicts_stay_in_tenant($setup().await).await;
        }
        #[tokio::test]
        async fn same_tenant_conflicts_update_only_selected_columns() {
            support::same_tenant_conflicts_update_only_selected_columns($setup().await).await;
        }
        #[tokio::test]
        async fn rejected_conflicts_never_reselect_a_row() {
            support::rejected_conflicts_never_reselect_a_row($setup().await).await;
        }
        #[tokio::test]
        async fn conflict_locks_check_expected_versions() {
            support::conflict_locks_check_expected_versions($setup().await).await;
        }
        #[tokio::test]
        async fn conflict_columns_are_validated_before_writing() {
            support::conflict_columns_are_validated_before_writing($setup().await).await;
        }
    };
}
pub(crate) use cases;
