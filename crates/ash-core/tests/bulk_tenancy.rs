//! Bulk creates take the same steps as single creates: they join the context's tenant
//! and run the hooks their action's changes register.

use ash_core::{BulkCreateOptions, Context, Error, FieldMap, Result, Value, resource};
use ash_memory::Memory;
use uuid::Uuid;

fn shout_label(fields: &mut FieldMap) -> Result<()> {
    if let Some(Value::String(label)) = fields.get_mut("label") {
        *label = label.to_uppercase();
    }
    Ok(())
}

resource! {
    Crate {
        table "crates";

        multitenancy {
            strategy: attribute;
            attribute: org;
        }

        attributes {
            id: Uuid [pk];
            org: String;
            label: String;
        }

        actions {
            create pack {
                primary;
                accept [label];
                change before_action(shout_label);
            }
            read read { primary; }
        }
    }
}

fn labels(labels: &[&str]) -> Vec<FieldMap> {
    labels
        .iter()
        .map(|label| {
            let mut fields = FieldMap::new();
            fields.insert("label".into(), Value::String((*label).into()));
            fields
        })
        .collect()
}

#[tokio::test]
async fn bulk_creates_join_the_tenant() {
    let ctx = Context::new(Memory::new());
    let acme = ctx.clone().with_tenant("acme");
    let packed = ash_core::bulk_create::<Crate, _, _, _>(
        &acme,
        "pack",
        labels(&["a", "b"]),
        BulkCreateOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(packed.count, 2);
    let packed = Crate::query(&acme).sort(Crate::label).all().await.unwrap();
    assert!(packed.iter().all(|c| c.org == "acme"));
    let shouted: Vec<&str> = packed.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        shouted,
        ["A", "B"],
        "the before_action hook ran for each row"
    );
    assert!(
        Crate::query(&ctx.clone().with_tenant("globex"))
            .all()
            .await
            .unwrap()
            .is_empty()
    );

    let err = ash_core::bulk_create::<Crate, _, _, _>(
        &ctx,
        "pack",
        labels(&["c"]),
        BulkCreateOptions::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::TenantRequired { .. }), "{err:?}");
}
