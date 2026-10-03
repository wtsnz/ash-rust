//! Validations fail as Ash's do: each with its message as a template and its vars
//! (`must have length of between %{min} and %{max}`), and every one that fails
//! reported, a create's and an update's alike.

use ash_core::{Context, Error, Value, resource};
use ash_memory::Memory;
use uuid::Uuid;

resource! {
    Item {
        table "validated_items";

        attributes {
            id: Uuid [pk];
            name: String;
            qty: i64;
            kind: String;
        }

        actions {
            create create {
                primary;
                accept [name, qty, kind];
                validate string_length(name, min: 3, max: 10);
                validate numericality(qty, min: 1, max: 5);
                validate one_of(kind, ["box", "crate"]);
            }

            update change {
                primary;
                accept [name, qty];
                validate string_length(name, min: 3, max: 10);
                validate numericality(qty, min: 1, max: 5);
            }
        }
    }
}

fn messages(err: &Error) -> Vec<(String, String)> {
    err.each()
        .into_iter()
        .map(|err| match err {
            Error::Validation { field, .. } => (field.clone(), err.message()),
            other => panic!("expected validation errors, got {other:?}"),
        })
        .collect()
}

#[tokio::test]
async fn every_failed_validation_is_reported_with_its_template_and_vars() {
    let ctx = Context::new(Memory::new());
    let err = Item::create(&ctx).name("x").qty(9).kind("bag").await.unwrap_err();
    assert!(matches!(err, Error::Multiple(_)), "{err:?}");
    assert_eq!(
        messages(&err),
        [
            ("name".into(), "must have length of between 3 and 10".into()),
            ("qty".into(), "must be greater than or equal to 1 and must be less than or equal to 5".into()),
            ("kind".into(), "expected one of box, crate".into()),
        ]
    );
    match err.each()[0] {
        Error::Validation { message, vars, .. } => {
            assert_eq!(message, "must have length of between %{min} and %{max}");
            assert_eq!(vars, &[("min".to_string(), Value::Int(3)), ("max".to_string(), Value::Int(10))]);
        }
        other => panic!("{other:?}"),
    }

    // One failure is itself, not a list of one.
    let one = Item::create(&ctx).name("fine").qty(9).kind("box").await.unwrap_err();
    assert!(matches!(one, Error::Validation { ref field, .. } if field == "qty"), "{one:?}");

    // An update, which plans its statement, reports every failure too.
    let item = Item::create(&ctx).name("fine").qty(2).kind("box").await.unwrap();
    let err = item.change(&ctx).name("no").qty(0).await.unwrap_err();
    assert_eq!(messages(&err).len(), 2, "{err:?}");
}
