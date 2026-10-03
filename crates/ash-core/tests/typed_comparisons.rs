use ash_core::{
    CompiledQuery, DataLayer, Error, FieldMap, Filter, Resource, Sort, Value, resource,
};
use ash_memory::Memory;
use uuid::Uuid;

resource! {
    Reading {
        table "readings";

        attributes {
            id: Uuid [pk];
            weight: ash_core::Float;
            amount: ash_core::Decimal;
            email: ash_core::CiString;
        }

        identities {
            identity unique_email: [email];
        }

        actions {
            read read { primary; }
        }
    }
}

fn row(n: u128, weight: &str, amount: &str, email: &str) -> (Uuid, FieldMap) {
    let id = Uuid::from_u128(n);
    let mut fields = FieldMap::new();
    fields.insert("id".into(), Value::Uuid(id));
    fields.insert("weight".into(), Value::String(weight.into()));
    fields.insert("amount".into(), Value::String(amount.into()));
    fields.insert("email".into(), Value::String(email.into()));
    (id, fields)
}

async fn ids(memory: &Memory, filter: Option<Filter>, sort: Vec<Sort>) -> Vec<u128> {
    let query = CompiledQuery {
        filter,
        sort,
        ..Default::default()
    };
    memory
        .run_query(&Reading::DEF, &query)
        .await
        .unwrap()
        .iter()
        .map(|row| match row.get("id") {
            Some(Value::Uuid(id)) => id.as_u128(),
            other => panic!("unexpected id {other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn memory_compares_like_the_column_type() {
    let memory = Memory::new();
    for (n, weight, amount, email) in [
        (1, "9.5", "10.50", "Ada@Example.com"),
        (2, "10", "9.99", "grace@example.com"),
        (3, "100.25", "-3", "linus@example.org"),
    ] {
        let (id, fields) = row(n, weight, amount, email);
        memory.create(&Reading::DEF, None, id, fields).await.unwrap();
    }

    let mut over = ids(&memory, Some(Filter::gt("weight", "9.6")), vec![]).await;
    over.sort();
    assert_eq!(over, [2, 3], "Float compares numerically, not as text");
    let by_weight = Sort {
        field: "weight".into(),
        descending: false,
        guard: None,
    };
    assert_eq!(ids(&memory, None, vec![by_weight]).await, [1, 2, 3]);
    assert_eq!(ids(&memory, Some(Filter::eq("weight", "9.50")), vec![]).await, [1]);
    assert_eq!(ids(&memory, Some(Filter::gt("amount", "9.995")), vec![]).await, [1]);
    assert_eq!(
        ids(&memory, Some(Filter::eq("email", "ADA@EXAMPLE.COM")), vec![]).await,
        [1]
    );
    assert_eq!(
        ids(
            &memory,
            Some(Filter::in_list("email", [Value::String("GRACE@example.com".into())])),
            vec![]
        )
        .await,
        [2]
    );

    let (id, duplicate) = row(4, "1", "0", "ADA@example.COM");
    let result = memory.create(&Reading::DEF, None, id, duplicate).await;
    assert!(
        matches!(result, Err(Error::IdentityConflict { .. })),
        "CiString identities ignore case: {result:?}"
    );
}

mod host {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Host {
            table "hosts";

            attributes {
                id: Uuid [pk];
                address: ash_core::Inet;
                embedding: ash_core::Vector<2>;
            }

            actions {
                create create { primary; accept [address, embedding]; }
                read read { primary; }
            }
        }
    }
}
use host::Host;

#[tokio::test]
async fn untyped_writes_store_canonical_text_and_filters_match_any_spelling() {
    use ash_core::{Changeset, Context};

    let ctx = Context::new(Memory::new());
    let mut input = FieldMap::new();
    input.insert("address".into(), Value::String("10.0.0.1/32".into()));
    input.insert("embedding".into(), Value::String("[1.0, 2.50]".into()));
    let host = Changeset::<Host>::for_create(&ctx, "create", input)
        .unwrap()
        .commit(&ctx)
        .await
        .unwrap();
    assert_eq!(host.address.as_str(), "10.0.0.1");
    assert_eq!(ash_core::format_vector(host.embedding.as_slice()), "[1,2.5]");

    let stored = ctx
        .data
        .run_query(&Host::DEF, &CompiledQuery::default())
        .await
        .unwrap();
    assert_eq!(stored[0].get("address"), Some(&Value::String("10.0.0.1".into())));
    assert_eq!(stored[0].get("embedding"), Some(&Value::String("[1,2.5]".into())));

    for filter in [
        Filter::eq("address", "10.0.0.1/32"),
        Filter::eq("embedding", "[1, 2.5]"),
    ] {
        let query = CompiledQuery {
            filter: Some(filter.clone()),
            ..Default::default()
        };
        let found = ctx.data.run_query(&Host::DEF, &query).await.unwrap();
        assert_eq!(found.len(), 1, "{filter:?}");
    }
}
