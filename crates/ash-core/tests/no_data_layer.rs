//! `build_<action>()` builds records against `NoDataLayer`, so defining resources needs
//! no real data layer, and persisting through it fails with a clear message.
//! `build_records.rs` covers what `build()` itself produces.

use ash_core::{Error, NoDataLayer, Resource, resource};
use uuid::Uuid;

resource! {
    Customer {
        table "customers";

        attributes {
            id: Uuid [pk];
            name: String;
        }

        identities {
            identity unique_name: [name];
        }

        actions {
            create register {
                primary;
                accept [name];
            }

            read read {
                primary;
            }
        }
    }
}

mod address {
    use ash_core::resource;

    resource! {
        embedded Address {
            attributes {
                city: String;
            }

            actions {
                create create {
                    primary;
                    accept [city];
                }
            }
        }
    }
}
use address::Address;

fn assert_no_data_layer(err: Error) {
    assert!(
        err.to_string()
            .contains("Customer has no data layer here; use a Context with a real data layer"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn build_makes_a_record_but_cannot_persist_it() {
    let built = Address::build_create().city("London").build().unwrap();
    assert_eq!(built.city, "London");

    let err = Customer::build_register().name("Ada").await.unwrap_err();
    assert_no_data_layer(err);
}

#[tokio::test]
async fn every_operation_reports_the_missing_data_layer() {
    let ctx = ash_core::Runtime::new(NoDataLayer).system().bootstrap();
    let id = Uuid::new_v4();
    let fields = ash_core::fields! { "id" => id, "name" => "Ada" };
    assert_no_data_layer(Customer::register(&ctx).name("Ada").await.unwrap_err());
    static UPDATE: ash_core::ActionDef = ash_core::ActionDef::update("rename").accept(&["name"]);
    static DESTROY: ash_core::ActionDef = ash_core::ActionDef::destroy("remove");
    assert_no_data_layer(
        ash_core::DynamicChangeset::for_update(
            &ctx,
            &Customer::DEF,
            &UPDATE,
            fields.clone(),
            ash_core::fields! { "name" => "Grace" },
        )
        .unwrap()
        .commit(&ctx)
        .await
        .unwrap_err(),
    );
    assert_no_data_layer(
        ash_core::DynamicChangeset::for_destroy(&ctx, &Customer::DEF, &DESTROY, fields.clone())
            .unwrap()
            .commit(&ctx)
            .await
            .unwrap_err(),
    );
    assert_no_data_layer(Customer::query(&ctx).all().await.unwrap_err());
    assert_no_data_layer(
        ash_core::DynamicChangeset::for_create_named(
            &ctx,
            &Customer::DEF,
            "register",
            ash_core::fields! { "name" => "Ada" },
        )
        .unwrap()
        .with_upsert("unique_name", &[])
        .commit(&ctx)
        .await
        .unwrap_err(),
    );
}
