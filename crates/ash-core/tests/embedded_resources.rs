use ash_core::{Context, Error, Resource, SchemaSupport, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

pub mod address {
    use super::*;

    // 1. Define an Embedded Resource (no table, no PK required)
    resource! {
        embedded;
        resource Address;

        attributes {
            street: String,
            city: String,
            zip_code: String,
        }

        actions {
            create create {
                primary;
                accept [street, city, zip_code];
                validate present(street);
                validate present(zip_code);
                validate string_length(street, min: 3);
            }

            update update {
                primary;
                accept [street, city, zip_code];
                validate present(street);
            }
        }
    }
}
pub use address::Address;

pub mod customer {
    use super::*;

    // 2. Define a Parent Resource that embeds Address as an attribute
    resource! {
        resource Customer;
        table "customers";

        attributes {
            id: Uuid [pk],
            name: String,
            address: Option<Address>,
        }

        actions {
            create register {
                primary;
                accept [name, address];
            }

            read read {
                primary;
            }

            update update_address {
                accept [address];
            }
        }
    }
}
pub use customer::Customer;

#[test]
fn test_embedded_standalone_build_and_validations() {
    // 1. Success validation & in-memory creation without DB or context
    let addr = Address::build_create()
        .street("100 Market St")
        .city("San Francisco")
        .zip_code("94105")
        .build()
        .expect("valid address should build");

    assert_eq!(addr.street, "100 Market St");
    assert_eq!(addr.city, "San Francisco");
    assert_eq!(addr.zip_code, "94105");

    // 2. Validation failure on string length
    let err_len = Address::build_create()
        .street("Ab") // < 3 characters
        .city("SF")
        .zip_code("94105")
        .build();
    assert!(err_len.is_err());
    assert!(matches!(err_len.unwrap_err(), Error::Validation { field, .. } if field == "street"));

    // 3. Validation failure on missing field
    let err_missing = Address::build_create()
        .street("Valid Street")
        .city("SF")
        // missing zip_code!
        .build();
    assert!(err_missing.is_err());

    // 4. Standalone update on existing embedded resource
    let updated = addr
        .build_update()
        .city("Oakland")
        .build()
        .expect("update should succeed");

    assert_eq!(updated.street, "100 Market St"); // preserved
    assert_eq!(updated.city, "Oakland");        // updated
    assert_eq!(updated.zip_code, "94105");       // preserved
}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_embedded_in_parent_resource_memory() {
    let ctx = Context::new(Memory::new());

    let addr = Address::build_create()
        .street("742 Evergreen Terrace")
        .city("Springfield")
        .zip_code("80085")
        .build()
        .expect("build address");

    // Create customer with embedded address
    let customer = Customer::register(&ctx)
        .name("Homer Simpson")
        .address(addr)
        .await
        .expect("create customer in memory");

    assert_eq!(customer.name, "Homer Simpson");
    assert!(customer.address.is_some());
    let customer_addr = customer.address.as_ref().unwrap();
    assert_eq!(customer_addr.street, "742 Evergreen Terrace");
    assert_eq!(customer_addr.city, "Springfield");

    // Query customer back
    let fetched = Customer::get(&ctx, customer.id).await.expect("fetch customer");
    let fetched_addr = fetched.address.expect("address present");
    assert_eq!(fetched_addr.street, "742 Evergreen Terrace");
    assert_eq!(fetched_addr.city, "Springfield");
}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_embedded_in_parent_resource_sqlite_json_roundtrip() {
    let db = Sqlite::memory().await.expect("sqlite memory");
    db.install_resources(&[&Customer::DEF]).await.expect("install schema");
    let ctx = Context::new(db);

    let addr = Address::build_create()
        .street("221B Baker Street")
        .city("London")
        .zip_code("NW1 6XE")
        .build()
        .expect("build address");

    // Create customer with embedded address persisted into SQLite TEXT as JSON
    let customer = Customer::register(&ctx)
        .name("Sherlock Holmes")
        .address(addr)
        .await
        .expect("create customer in sqlite");

    assert_eq!(customer.name, "Sherlock Holmes");
    assert!(customer.address.is_some());
    assert_eq!(customer.address.as_ref().unwrap().city, "London");

    // Fetch back from SQLite and verify JSON deserialized into typed Address struct
    let fetched = Customer::get(&ctx, customer.id).await.expect("fetch from sqlite");
    let fetched_addr = fetched.address.expect("address present");
    assert_eq!(fetched_addr.street, "221B Baker Street");
    assert_eq!(fetched_addr.city, "London");
    assert_eq!(fetched_addr.zip_code, "NW1 6XE");

    // Update embedded address on existing record
    let new_addr = fetched_addr
        .build_update()
        .street("10 Downing Street")
        .city("London")
        .zip_code("SW1A 2AA")
        .build()
        .expect("build update");

    let updated_cust = customer
        .update_address_on(&ctx)
        .address(new_addr)
        .await
        .expect("update address in sqlite");

    let re_read = Customer::get(&ctx, updated_cust.id).await.expect("re-read from sqlite");
    assert_eq!(re_read.address.as_ref().unwrap().street, "10 Downing Street");
    assert_eq!(re_read.address.as_ref().unwrap().zip_code, "SW1A 2AA");
}
