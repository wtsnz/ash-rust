use ash_core::{AttrType, Context, Error, FieldMap, Resource, Value, resource};
use ash_memory::Memory;
use uuid::Uuid;

resource! {
RefundRequest {
    table "refund_requests";

attributes {
    id: Uuid [pk];
    order_id: String;
    amount: i64;
    status: String;
    reason: Option<String>;
}

actions {
    create submit {
        primary;
        accept [order_id, amount];
        argument reason: String;
        validate present(reason);
        validate string_length(reason, min: 5);
        change set(status = "submitted");
        change set_from_arg(reason, reason);
    }

    update approve {
        argument note: String;
        validate present(note);
        change set(status = "approved");
    }
}
}}

#[tokio::test]
async fn test_action_arguments_success_and_set_from_arg() {
    let ctx = Context::new(Memory::new());

    // Submit a refund request with required argument `reason`
    let refund = RefundRequest::submit(&ctx)
        .order_id("ORD-1234")
        .amount(99)
        .reason("Item arrived damaged")
        .await
        .expect("should submit successfully");

    assert_eq!(refund.order_id, "ORD-1234");
    assert_eq!(refund.amount, 99);
    assert_eq!(refund.status, "submitted");
    assert_eq!(refund.reason, Some("Item arrived damaged".into()));
}

#[tokio::test]
async fn test_action_arguments_validation_failure() {
    let ctx = Context::new(Memory::new());

    // Providing reason with fewer than 5 characters should fail validation
    let err = RefundRequest::submit(&ctx)
        .order_id("ORD-1234")
        .amount(99)
        .reason("bad")
        .await
        .expect_err("should fail string_length validation on argument");

    let text = err.message();
    match err {
        Error::Validation { field, .. } => {
            assert_eq!(field, "reason");
            assert_eq!(text, "must have length of at least 5");
        }
        other => panic!("expected Error::Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn test_action_arguments_on_update() {
    let ctx = Context::new(Memory::new());

    let refund = RefundRequest::submit(&ctx)
        .order_id("ORD-999")
        .amount(50)
        .reason("Defective part inside")
        .await
        .unwrap();

    // Update with required `note` argument
    let approved = refund
        .approve_on(&ctx)
        .note("Verified receipt and warehouse return")
        .await
        .expect("should approve");

    assert_eq!(approved.status, "approved");
}

// The action only declares its arguments: nothing uses them.
#[allow(deprecated)]
mod shipments {
    use ash_core::{FieldMap, resource};
    use uuid::Uuid;

resource! {
Shipment {
    table "shipments";

attributes {
    id: Uuid [pk];
    weight: i64;
}

calculations {
    scaled(factor: f64, label: Option<String>): i64 = weight;
}

actions {
    create pack {
        accept [weight];
        argument items: Vec<FieldMap>;
        argument options: Option<FieldMap>;
        argument rate: f64;
        argument fragile: bool;
    }
}
}}
}
use shipments::Shipment;

// An argument has the type it's declared with, as an attribute does: a list, a map, a
// float. Each used to be declared as text whatever its type, so a client that sent a list
// or a map, through any API that casts by the argument's type, was refused.
#[test]
fn test_arguments_take_their_declared_types() {
    let pack = Shipment::DEF.action("pack").unwrap();
    let ty = |name: &str| pack.arguments.iter().find(|arg| arg.name == name).map(|arg| (arg.ty, arg.allow_nil));
    assert_eq!(ty("items"), Some((AttrType::Array, false)));
    assert_eq!(ty("options"), Some((AttrType::Map, true)));
    assert_eq!(ty("rate"), Some((AttrType::Float, false)));
    assert_eq!(ty("fragile"), Some((AttrType::Boolean, false)));

    let scaled = Shipment::DEF.calculation("scaled").unwrap();
    let types: Vec<_> = scaled.arguments.iter().map(|arg| (arg.name, arg.ty, arg.allow_nil)).collect();
    assert_eq!(types, [("factor", AttrType::Float, false), ("label", AttrType::String, true)]);
}

#[tokio::test]
async fn test_list_argument_reaches_the_action() {
    let ctx = Context::new(Memory::new());
    let mut item = FieldMap::new();
    item.insert("sku".into(), Value::String("A-1".into()));
    let shipment = Shipment::pack(&ctx).weight(3).items(vec![item]).rate(1.5).fragile(true).await;
    assert!(shipment.is_ok(), "{shipment:?}");
}
