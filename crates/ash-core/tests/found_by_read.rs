//! An update or destroy by id finds its record through a read, the primary one by
//! default or one given, as an RPC action's `read_action` names it; and a destroy takes
//! input, its arguments for its changes and validations.

use ash_core::{Context, DataLayer, Error, FieldMap, Resource, Value, destroy_dynamic_via, update_dynamic, update_dynamic_via};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

mod tickets {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Ticket {
            table "found_tickets";

            attributes {
                id: Uuid [pk];
                title: String;
                archived: bool [default: false];
            }

            actions {
                read read {
                    primary;
                    prepare filter(archived == false);
                }
                read everything {}
                create create { primary; accept [title, archived]; }
                update rename { accept [title]; }
                destroy remove {
                    argument reason: String;
                    validate string_length(reason, min: 3);
                }
            }
        }
    }
}

use tickets::Ticket;

fn input(pairs: &[(&str, Value)]) -> FieldMap {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

async fn scenario<D: DataLayer + Clone + 'static>(data: D) {
    let ctx = Context::new(data);
    let def = &Ticket::DEF;
    let action = |name: &str| def.action(name).unwrap();
    let archived = Ticket::create(&ctx).title("old").archived(true).await.unwrap().id;
    let rename = || input(&[("title", Value::from("renamed"))]);

    // The primary read hides an archived ticket; one that reads every ticket finds it.
    let hidden = update_dynamic(&ctx, def, action("rename"), archived, rename()).await;
    assert!(matches!(hidden, Err(Error::NotFound)), "{hidden:?}");
    let renamed = update_dynamic_via(&ctx, def, Some(action("everything")), action("rename"), archived, rename())
        .await
        .unwrap();
    assert_eq!(renamed.get("title"), Some(&Value::from("renamed")));

    // A destroy's input reaches its validations.
    let everything = Some(action("everything"));
    let short = destroy_dynamic_via(&ctx, def, everything, action("remove"), archived, input(&[("reason", Value::from("no"))])).await;
    assert!(matches!(short, Err(Error::Validation { ref field, .. }) if field == "reason"), "{short:?}");
    let missing = destroy_dynamic_via(&ctx, def, everything, action("remove"), archived, FieldMap::new()).await;
    assert!(matches!(missing, Err(Error::Missing { ref field }) if field == "reason"), "{missing:?}");
    destroy_dynamic_via(&ctx, def, everything, action("remove"), archived, input(&[("reason", Value::from("duplicate"))]))
        .await
        .unwrap();
    let gone = destroy_dynamic_via(&ctx, def, everything, action("remove"), Uuid::new_v4(), input(&[("reason", Value::from("gone"))])).await;
    assert!(matches!(gone, Err(Error::NotFound)), "{gone:?}");
}

#[tokio::test]
async fn found_by_read_in_memory() {
    scenario(Memory::new()).await;
}

#[tokio::test]
async fn found_by_read_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Ticket::DEF]).await.unwrap();
    scenario(sqlite).await;
}
