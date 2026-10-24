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
