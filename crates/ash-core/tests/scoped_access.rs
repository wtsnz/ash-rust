use ash_core::{Actor, Error, FieldMap, Filter, ReadRequest, Resource};
use ash_memory::Memory;
use uuid::Uuid;

ash_core::resource! {
    Account {
        table "scoped_access_accounts";
        actor { role: String; }
        multitenancy { strategy: attribute; attribute: org; }
        attributes {
            id: Uuid [pk];
            org: String;
            hash: String;
        }
        policies {
            policy always { authorize_if actor_attribute_equals(role, "admin"); }
        }
        actions {
            create create { primary; accept [hash]; }
            read read { primary; }
            read prepared {
                argument hash: String;
                prepare filter(hash == "a-hash");
                prepare after_action(mark_read);
            }
        }
    }
}

fn mark_read(_arguments: &FieldMap, rows: &mut [ash_core::RawRecord]) -> ash_core::Result<()> {
    for row in rows {
        ash_core::put_metadata(row.fields_mut(), "read_hook", true);
    }
    Ok(())
}

#[tokio::test]
async fn scoped_reads_apply_named_action_preparations_and_hooks() {
    let ctx = ash_core::Runtime::new(Memory::new()).bind(
        Some(Actor::new(Uuid::new_v4()).with_role("admin")),
        Some(("a").into()),
    );
    Account::create(&ctx).hash("a-hash").await.unwrap();
    Account::create(&ctx).hash("other-hash").await.unwrap();
    let access = ctx.access();
    let rows = access
        .read(
            &Account::DEF,
            "prepared",
            &FieldMap::from([("hash".into(), "a-hash".into())]),
            ReadRequest::default(),
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("hash").cloned().unwrap(), "a-hash".into());
    assert_eq!(
        rows[0]
            .metadata(&ash_core::MetadataDef {
                name: "read_hook",
                ty: ash_core::AttrType::Boolean,
                allow_nil: false
            })
            .unwrap()
            .map(|value| value.to_value()),
        Some(true.into())
    );
    let rows = access
        .read(
            &Account::DEF,
            "prepared",
            &FieldMap::from([("hash".into(), "other-hash".into())]),
            ReadRequest::default(),
        )
        .await
        .unwrap();
    assert!(rows.is_empty());
    assert!(matches!(
        access
            .read(
                &Account::DEF,
                "create",
                &FieldMap::new(),
                ReadRequest::default()
            )
            .await,
        Err(Error::WrongActionKind { .. })
    ));
    assert!(matches!(
        access
            .read(
                &Account::DEF,
                "missing",
                &FieldMap::new(),
                ReadRequest::default()
            )
            .await,
        Err(Error::UnknownAction { .. })
    ));
}

#[tokio::test]
async fn scoped_reads_and_credential_exception_keep_the_tenant() {
    let root_runtime = ash_core::Runtime::new(Memory::new());
    let root = root_runtime.bootstrap();
    let admin = root_runtime.bind(
        Some(Actor::new(Uuid::new_v4()).with_role("admin")),
        root.tenant().map(str::to_owned),
    );
    let a = root_runtime.bind(admin.actor().cloned(), Some("a".into()));
    let b = root_runtime.bind(admin.actor().cloned(), Some("b".into()));
    Account::create(&a).hash("a-hash").await.unwrap();
    Account::create(&b).hash("b-hash").await.unwrap();
    let rows = a
        .access()
        .read(
            &Account::DEF,
            "read",
            &FieldMap::new(),
            ReadRequest::default(),
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("hash").cloned().unwrap(), "a-hash".into());
    let rows = a
        .access()
        .read(
            &Account::DEF,
            "read",
            &FieldMap::new(),
            ReadRequest {
                filter: Some(Filter::eq("org", "b")),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(rows.is_empty());

    let anonymous = root_runtime.bind(root.actor().cloned(), Some(("a").into()));
    match anonymous
        .access()
        .read(
            &Account::DEF,
            "read",
            &FieldMap::new(),
            ReadRequest::default(),
        )
        .await
    {
        Err(Error::Forbidden) => {}
        Ok(rows) => assert!(rows.is_empty()),
        Err(error) => panic!("unexpected read failure: {error}"),
    }
    let permit = root_runtime.credential_reads::<Account>();
    let rows = anonymous
        .access()
        .credentials(&permit, Filter::True, 2)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("hash").cloned().unwrap(), "a-hash".into());
    assert!(matches!(
        root.access().credentials(&permit, Filter::True, 2).await,
        Err(Error::TenantRequired { .. })
    ));
}

#[test]
fn signed_tenants_must_match_exactly() {
    let root_runtime = ash_core::Runtime::new(Memory::new());
    let root = root_runtime.bootstrap();
    assert!(root.access().require_tenant_match(None).is_ok());
    assert!(root.access().require_tenant_match(Some("a")).is_err());
    let a = root_runtime.bind(root.actor().cloned(), Some(("a").into()));
    assert!(a.access().require_tenant_match(Some("a")).is_ok());
    assert!(a.access().require_tenant_match(Some("b")).is_err());
    assert!(a.access().require_tenant_match(None).is_err());
}

#[tokio::test]
async fn credential_grants_are_bound_to_the_issuing_runtime() {
    let runtime = ash_core::Runtime::new(Memory::new());
    let admin = runtime.system().bind(None, Some("a".into()));
    let account = Account::create(&admin).hash("unchanged").await.unwrap();
    let guest = runtime.bind(None, Some("a".into()));
    let unrelated = ash_core::Runtime::new(());
    assert!(matches!(
        guest
            .access()
            .credentials(&unrelated.credential_reads::<Account>(), Filter::True, 1)
            .await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        guest
            .access()
            .update_credentials(
                &unrelated.credential_writes::<Account>(["hash"]),
                account.id.into(),
                ash_core::fields! { "hash" => "foreign-grant" }
            )
            .await,
        Err(Error::Forbidden)
    ));
    assert_eq!(
        Account::get(&admin, account.id).await.unwrap().hash,
        "unchanged"
    );

    let read = runtime.clone().credential_reads::<Account>();
    let write = runtime.clone().credential_writes::<Account>(["hash"]);
    runtime
        .transaction(|tx| async move {
            let ctx = tx.bind(None, Some("a".into()));
            let rows = ctx.access().credentials(&read, Filter::True, 1).await?;
            assert_eq!(rows[0].get("hash"), Some(&"unchanged".into()));
            ctx.access()
                .update_credentials(
                    &write,
                    account.id.into(),
                    ash_core::fields! { "hash" => "same-runtime" },
                )
                .await?;
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        Account::get(&admin, account.id).await.unwrap().hash,
        "same-runtime"
    );
}

async fn credential_mutations<D: ash_core::DataLayer>(runtime: ash_core::Runtime<D>) {
    let permit = runtime.credential_writes::<Account>(["hash", "id", "org"]);
    let a = runtime.system().bind(None, Some("a".into()));
    let b = runtime.system().bind(None, Some("b".into()));
    let own = Account::create(&a).hash("old").await.unwrap();
    let other = Account::create(&b).hash("other").await.unwrap();
    let guest = runtime.bind(None, Some("a".into()));
    let updated = guest
        .access()
        .update_credentials(
            &permit,
            own.id.into(),
            ash_core::fields! { "hash" => "new" },
        )
        .await
        .unwrap();
    assert_eq!(updated.get("hash"), Some(&"new".into()));
    assert!(matches!(
        guest
            .access()
            .update_credentials(
                &permit,
                other.id.into(),
                ash_core::fields! { "hash" => "stolen" }
            )
            .await,
        Err(Error::NotFound)
    ));
    for fields in [
        ash_core::fields! { "id" => other.id },
        ash_core::fields! { "org" => "b" },
        ash_core::fields! { "hash" => 42 },
    ] {
        assert!(
            guest
                .access()
                .update_credentials(&permit, own.id.into(), fields)
                .await
                .is_err()
        );
    }
    assert!(matches!(
        runtime
            .bootstrap()
            .access()
            .update_credentials(
                &permit,
                own.id.into(),
                ash_core::fields! { "hash" => "missing-tenant" }
            )
            .await,
        Err(Error::TenantRequired { .. })
    ));
    assert_eq!(Account::get(&a, own.id).await.unwrap().hash, "new");
    assert_eq!(Account::get(&b, other.id).await.unwrap().hash, "other");
}

#[tokio::test]
async fn credential_write_grants_restrict_fields_and_tenants_on_memory() {
    credential_mutations(ash_core::Runtime::new(Memory::new())).await;
}

#[tokio::test]
async fn credential_write_grants_restrict_fields_and_tenants_on_sqlite() {
    let data = ash_sqlite::Sqlite::memory().await.unwrap();
    data.install(&[&Account::DEF]).await.unwrap();
    credential_mutations(ash_core::Runtime::new(data)).await;
}

#[tokio::test]
async fn prepared_reads_keep_their_scope_and_candidate_predicate() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static PREPARATIONS: AtomicUsize = AtomicUsize::new(0);
    fn prepared_filter() -> ash_core::TrustedExpr {
        PREPARATIONS.fetch_add(1, Ordering::SeqCst);
        ash_core::TrustedExpr::Filter(Filter::eq("hash", "visible"))
    }
    static ACTIONS: &[ash_core::ActionDef] = &[ash_core::ActionDef::read("once")
        .preparations(&[ash_core::PreparationDef::Filter(prepared_filter)])];
    let runtime = ash_core::Runtime::new(Memory::new());
    let a = runtime.system().bind(None, Some("a".into()));
    let b = runtime.system().bind(None, Some("b".into()));
    Account::create(&a).hash("visible").await.unwrap();
    Account::create(&a).hash("hidden").await.unwrap();
    Account::create(&b).hash("visible").await.unwrap();
    let resource = ash_core::ResourceDef {
        actions: ACTIONS,
        ..Account::DEF
    };
    let read = a
        .access()
        .prepare_read(&resource, "once", &FieldMap::new(), ReadRequest::default())
        .unwrap();
    let shape = || ReadRequest {
        filter: Some(Filter::True),
        ..Default::default()
    };
    let (rows, count) = a
        .access()
        .read_prepared_with_count(&read, shape(), shape())
        .await
        .unwrap();
    assert_eq!((rows.len(), count), (1, 1));
    assert_eq!(rows[0].get("hash"), Some(&"visible".into()));
    assert_eq!(PREPARATIONS.load(Ordering::SeqCst), 1);
    assert!(b.access().read_prepared(&read, shape()).await.is_err());
    let rebound = runtime.system().bind(None, Some("a".into()));
    assert!(
        rebound
            .access()
            .count_prepared(&read, shape())
            .await
            .is_err()
    );
    a.transaction(|tx| async move {
        assert_eq!(tx.access().read_prepared(&read, shape()).await?.len(), 1);
        Ok(())
    })
    .await
    .unwrap();
}
