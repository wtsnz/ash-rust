use ash_core::{
    BulkDestroyOptions, CompiledQuery, Context, DataLayer, Filter, Resource, ResourceExt,
    TransactionSupport, Value,
    bulk_destroy, destroy_existing, resource,
};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

pub mod folder_mod {
    use super::*;

    resource! {
        Folder {
            table "folders";

            attributes {
                id: Uuid [pk];
                name: String;
                archived_at: Option<String>;
            }

            relationships {
                has_many notes: note_mod::Note [fk: folder_id, on_delete: cascade];
            }

            actions {
                create create {
                    primary;
                    accept [name];
                }

                read read {
                    primary;
                }

                destroy archive {
                    primary;
                    soft;
                    cascade_destroy [notes];
                    change set(archived_at = ash_core::utc_now_iso8601());
                }

                destroy archive_only {
                    soft;
                    change set(archived_at = ash_core::utc_now_iso8601());
                }
            }
        }
    }
}

pub mod note_mod {
    use super::*;

    resource! {
        Note {
            table "notes";

            attributes {
                id: Uuid [pk];
                folder_id: Uuid;
                body: String;
                archived_at: Option<String>;
            }

            relationships {
                belongs_to folder: folder_mod::Folder [fk: folder_id];
            }

            actions {
                create create {
                    primary;
                    accept [folder_id, body];
                }

                read read {
                    primary;
                    prepare filter(archived_at.is_nil());
                }

                destroy destroy {
                    primary;
                    soft;
                    change set(archived_at = ash_core::utc_now_iso8601());
                }
            }
        }
    }
}

use folder_mod::Folder;
use note_mod::Note;

/// Every stored note, archived or not, straight from the data layer.
async fn stored_notes<D: DataLayer>(ctx: &Context<D>) -> Vec<(String, Option<String>)> {
    let mut rows: Vec<(String, Option<String>)> = ctx
        .data
        .run_query(&Note::DEF, &CompiledQuery::default())
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            let body = row.get("body").and_then(Value::as_str).unwrap().to_string();
            let archived = row.get("archived_at").and_then(Value::as_str).map(str::to_string);
            (body, archived)
        })
        .collect();
    rows.sort();
    rows
}

async fn scenario<D: DataLayer>(ctx: Context<D>) {
    let folder = Folder::create(&ctx).name("Inbox").await.unwrap();
    for body in ["first", "second", "third"] {
        Note::create(&ctx).folder_id(folder.id).body(body).await.unwrap();
    }

    // A soft destroy stores the change and keeps the row.
    let first = Note::query(&ctx)
        .filter(Note::body.eq("first"))
        .one()
        .await
        .unwrap();
    first.destroy(&ctx).await.unwrap();
    let stored = stored_notes(&ctx).await;
    assert_eq!(stored.len(), 3, "soft destroy must keep the row");
    assert!(stored[0].1.is_some(), "first note must be archived: {stored:?}");

    // The primary read filter hides it from queries and from relationship loads.
    assert_eq!(Note::query(&ctx).count().await.unwrap(), 2);
    let loaded = Folder::query(&ctx)
        .load_rel(Folder::notes)
        .one()
        .await
        .unwrap();
    let mut bodies: Vec<&str> = loaded.notes.loaded().unwrap().iter().map(|n| n.body.as_str()).collect();
    bodies.sort();
    assert_eq!(bodies, ["second", "third"]);

    // Soft-destroying the folder skips its on_delete cascade, since the row stays.
    let archived_at_first = stored[0].1.clone();
    destroy_existing::<Folder, D>(&ctx, "archive_only", folder).await.unwrap();
    let folder_row = Folder::query(&ctx).one().await.unwrap();
    assert!(folder_row.archived_at.is_some());
    assert_eq!(stored_notes(&ctx).await.iter().filter(|n| n.1.is_none()).count(), 2);

    // cascade_destroy runs each note's primary destroy, which archives it, and leaves
    // the note that was already archived alone.
    folder_row.destroy(&ctx).await.unwrap();
    let stored = stored_notes(&ctx).await;
    assert_eq!(stored.len(), 3);
    assert!(stored.iter().all(|n| n.1.is_some()), "{stored:?}");
    assert_eq!(stored[0].1, archived_at_first, "already archived notes are skipped");
}

async fn bulk_scenario<D: TransactionSupport + 'static>(ctx: Context<D>) {
    let folder = Folder::create(&ctx).name("Bulk").await.unwrap();
    let mut ids = Vec::new();
    for body in ["a", "b"] {
        ids.push(Note::create(&ctx).folder_id(folder.id).body(body).await.unwrap().id);
    }
    let result = bulk_destroy::<Note, D, _>(&ctx, "destroy", &ids, BulkDestroyOptions::default())
        .await
        .unwrap();
    assert_eq!(result.count, 2);
    assert!(result.records.iter().all(|n| n.archived_at.is_some()));
    assert_eq!(Note::query(&ctx).count().await.unwrap(), 0);
    assert_eq!(stored_notes(&ctx).await.len(), 2);
    assert_eq!(
        Note::query(&ctx)
            .filter(Filter::is_nil("archived_at"))
            .count()
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn soft_destroy_and_cascade_destroy_in_memory() {
    scenario(Context::new(Memory::new())).await;
    bulk_scenario(Context::new(Memory::new())).await;
}

#[tokio::test]
async fn soft_destroy_and_cascade_destroy_in_sqlite() {
    async fn sqlite() -> Context<Sqlite> {
        let sqlite = Sqlite::memory().await.unwrap();
        sqlite.install(&[&Folder::DEF, &Note::DEF]).await.unwrap();
        Context::new(sqlite)
    }
    scenario(sqlite().await).await;
    bulk_scenario(sqlite().await).await;
}
