use ash_core::{Context, Error, resource};
use ash_memory::Memory;
use uuid::Uuid;

resource! {
    resource Product;
    table "products";

    attributes {
        id: Uuid [pk],
        name: String,
        category: String,
        price: i64,
        status: String,
    }

    actions {
        create create {
            accept [name, category, price, status];
            validate present(name);
            validate string_length(name, min = 3, max = 20);
            validate one_of(category, ["electronics", "books", "clothing"]);
            validate numericality(price, min = 1, max = 10000);
            change set_new(status = "draft");
        }

        update publish {
            change set(status = "published");
        }

        read read {
            primary
        }
    }

    policies {
        policy always {
            authorize_if always
        }
    }
}

#[tokio::test]
async fn validation_present_and_string_length() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    // 1. Valid product creation succeeds
    let product = Product::create(&ctx)
        .name("Laptop")
        .category("electronics")
        .price(999)
        .await
        .expect("should create valid product");
    assert_eq!(product.name, "Laptop");
    assert_eq!(product.status, "draft");

    // 2. Empty name fails present validation
    let err = Product::create(&ctx)
        .name("")
        .category("electronics")
        .price(500)
        .await
        .unwrap_err();
    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "name");
            assert!(message.contains("must be present"), "unexpected message: {message}");
        }
        other => panic!("expected Error::Validation, got {:?}", other),
    }

    // 3. Whitespace name fails present validation
    let err = Product::create(&ctx)
        .name("   ")
        .category("electronics")
        .price(500)
        .await
        .unwrap_err();
    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "name");
            assert!(message.contains("must be present"), "unexpected message: {message}");
        }
        other => panic!("expected Error::Validation, got {:?}", other),
    }

    // 4. Too short name fails string_length min validation
    let err = Product::create(&ctx)
        .name("ab")
        .category("electronics")
        .price(500)
        .await
        .unwrap_err();
    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "name");
            assert!(message.contains("at least 3 characters"), "unexpected message: {message}");
        }
        other => panic!("expected Error::Validation, got {:?}", other),
    }

    // 5. Too long name fails string_length max validation
    let err = Product::create(&ctx)
        .name("this is a very long product name exceeding twenty characters")
        .category("electronics")
        .price(500)
        .await
        .unwrap_err();
    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "name");
            assert!(message.contains("at most 20 characters"), "unexpected message: {message}");
        }
        other => panic!("expected Error::Validation, got {:?}", other),
    }
}

#[tokio::test]
async fn validation_one_of() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    // 1. Allowed category succeeds
    let p = Product::create(&ctx)
        .name("Rust Book")
        .category("books")
        .price(50)
        .await
        .expect("valid category");
    assert_eq!(p.category, "books");

    // 2. Disallowed category fails
    let err = Product::create(&ctx)
        .name("Banana")
        .category("groceries")
        .price(5)
        .await
        .unwrap_err();
    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "category");
            assert!(message.contains("must be one of: electronics, books, clothing"), "unexpected: {message}");
        }
        other => panic!("expected Error::Validation, got {:?}", other),
    }
}

#[tokio::test]
async fn validation_numericality() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    // 1. Negative or zero price fails min
    let err = Product::create(&ctx)
        .name("Keyboard")
        .category("electronics")
        .price(0)
        .await
        .unwrap_err();
    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "price");
            assert!(message.contains("at least 1"), "unexpected: {message}");
        }
        other => panic!("expected Error::Validation, got {:?}", other),
    }

    // 2. Price exceeding 10000 fails max
    let err = Product::create(&ctx)
        .name("Supercomputer")
        .category("electronics")
        .price(50000)
        .await
        .unwrap_err();
    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "price");
            assert!(message.contains("at most 10000"), "unexpected: {message}");
        }
        other => panic!("expected Error::Validation, got {:?}", other),
    }
}

#[tokio::test]
async fn change_set_new_preserves_caller_value_or_defaults() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    // 1. Without providing status: set_new defaults to "draft"
    let p1 = Product::create(&ctx)
        .name("Monitor")
        .category("electronics")
        .price(300)
        .await
        .expect("created");
    assert_eq!(p1.status, "draft");

    // 2. With providing status: set_new preserves caller's status
    let p2 = Product::create(&ctx)
        .name("Smart Watch")
        .category("electronics")
        .price(250)
        .status("preorder")
        .await
        .expect("created with custom status");
    assert_eq!(p2.status, "preorder");

    // 3. Action change set overwrites
    let p2_published = p2.publish_on(&ctx).await.expect("published");
    assert_eq!(p2_published.status, "published");
}

#[tokio::test]
async fn helpdesk_ticket_and_representative_validations() {
    use helpdesk::{Helpdesk, Ticket, actor_customer};

    let mem = Memory::new();
    let desk = Helpdesk::new(mem);
    let as_customer = desk.with_actor(actor_customer(Uuid::new_v4()));

    // 1. Subject with 1 character fails string_length(min = 2)
    let err = Ticket::open(&as_customer)
        .subject("x")
        .await
        .unwrap_err();
    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "subject");
            assert!(message.contains("at least 2 characters"));
        }
        other => panic!("expected validation error, got {:?}", other),
    }

    // 2. Empty subject fails present validation
    let err = as_customer
        .open_ticket("")
        .await
        .unwrap_err();
    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "subject");
            assert!(message.contains("must be present"));
        }
        other => panic!("expected validation error, got {:?}", other),
    }

    // 3. Valid subject succeeds
    let ticket = as_customer
        .open_ticket("Wifi connection problem")
        .await
        .expect("valid ticket");
    assert_eq!(ticket.subject, "Wifi connection problem");
    assert_eq!(ticket.status, helpdesk::Status::Open);

    // 4. Representative name validation
    let err = desk
        .create_representative("A")
        .await
        .unwrap_err();
    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "name");
            assert!(message.contains("at least 2 characters"));
        }
        other => panic!("expected validation error, got {:?}", other),
    }
}
