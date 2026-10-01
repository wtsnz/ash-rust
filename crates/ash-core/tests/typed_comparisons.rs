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
        memory.create(&Reading::DEF, id, fields).await.unwrap();
    }

    let mut over = ids(&memory, Some(Filter::gt("weight", "9.6")), vec![]).await;
    over.sort();
    assert_eq!(over, [2, 3], "Float compares numerically, not as text");
    let by_weight = Sort {
        field: "weight".into(),
        descending: false,
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
    let result = memory.create(&Reading::DEF, id, duplicate).await;
    assert!(
        matches!(result, Err(Error::IdentityConflict { .. })),
        "CiString identities ignore case: {result:?}"
    );
}
