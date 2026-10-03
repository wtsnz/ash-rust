//! `build_<action>().build()` returns the record an action would store without saving
//! it, for resources with a table as well as embedded ones.

use ash_core::{Error, resource};
use uuid::Uuid;

resource! {
    Account {
        table "accounts";

        timestamps;

        attributes {
            id: Uuid [pk];
            name: String;
            plan: String = "free";
            version: i64 [version];
        }

        actions {
            create register {
                primary;
                accept [name];
                validate present(name);
            }

            update rename {
                primary;
                accept [name];
            }
        }
    }
}

#[test]
fn build_fills_in_what_saving_would() {
    let account = Account::build_register().name("Ada").build().unwrap();
    assert_ne!(account.id, Uuid::nil());
    assert_eq!(account.plan, "free");
    assert_eq!(account.version, 1);
    assert!(!account.created_at.as_str().is_empty());
    assert_eq!(account.created_at, account.updated_at);

    let other = Account::build_register().name("Grace").build().unwrap();
    assert_ne!(other.id, account.id, "each build gets its own id");
}

#[test]
fn build_runs_validations() {
    let err = Account::build_register().name("").build().unwrap_err();
    assert!(matches!(err, Error::Validation { .. }), "{err:?}");
}

#[test]
fn build_on_an_update_moves_the_version_on() {
    let account = Account::build_register().name("Ada").build().unwrap();
    let renamed = account.build_rename().name("Ada L").build().unwrap();
    assert_eq!(renamed.id, account.id);
    assert_eq!(renamed.name, "Ada L");
    assert_eq!(renamed.version, 2);
    assert_eq!(renamed.created_at, account.created_at);
}
