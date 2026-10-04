//! `build_<action>()` builds records against `NoDataLayer`, so defining resources needs
//! no real data layer, and persisting through it fails with a clear message.
//! `build_records.rs` covers what `build()` itself produces.

use ash_core::{DataLayer, Error, FieldMap, NoDataLayer, Resource, resource};
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
    let data = NoDataLayer;
    let def = &Customer::DEF;
    let identity = def.identity("unique_name").unwrap();
    let id = Uuid::new_v4();

    assert_no_data_layer(data.create(def, None, ash_core::Value::from(id), FieldMap::new()).await.unwrap_err());
    assert_no_data_layer(data.update(def, None, ash_core::Value::from(id), FieldMap::new()).await.unwrap_err());
    assert_no_data_layer(data.destroy(def, None, ash_core::Value::from(id)).await.unwrap_err());
    assert_no_data_layer(data.run_query(def, &Default::default()).await.unwrap_err());
    assert_no_data_layer(
        data.upsert(def, None, ash_core::Value::from(id), FieldMap::new(), identity, &[])
            .await
            .unwrap_err(),
    );
}
