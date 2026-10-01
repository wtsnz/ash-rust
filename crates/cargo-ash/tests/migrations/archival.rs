use ash_core::{Context, DataLayer, Resource, ResourceExt};

use crate::fixtures::archived_notes::ArchivedNote;
use crate::fixtures::note_folders::NoteFolder;
use crate::support::{Db, Project, TestDb, on_every_backend};

async fn archive_and_restore<D: DataLayer>(ctx: Context<D>) {
    let note = ArchivedNote::create(&ctx).title("draft").await.unwrap();
    note.destroy(&ctx).await.unwrap();
    assert_eq!(ArchivedNote::query(&ctx).count().await.unwrap(), 0);

    let archived = ArchivedNote::query(&ctx)
        .action("archived")
        .one()
        .await
        .unwrap();
    assert!(archived.is_archived());

    let restored = ash_archival::unarchive::<ArchivedNote, D>(&ctx, note.id)
        .await
        .unwrap();
    assert!(!restored.is_archived());
    assert_eq!(ArchivedNote::query(&ctx).count().await.unwrap(), 1);

    // Aggregates and filters through a relationship skip archived notes.
    let folder = NoteFolder::create(&ctx).name("inbox").await.unwrap();
    for title in ["live", "gone"] {
        ArchivedNote::create(&ctx)
            .title(title)
            .folder_id(Some(folder.id))
            .await
            .unwrap();
    }
    ArchivedNote::query(&ctx)
        .filter(ash_core::Filter::eq("title", "gone"))
        .one()
        .await
        .unwrap()
        .destroy(&ctx)
        .await
        .unwrap();
    let folder = NoteFolder::query(&ctx)
        .load_aggregate(NoteFolder::note_count)
        .one()
        .await
        .unwrap();
    assert_eq!(folder.note_count, Some(1));
    for (title, expected) in [("gone", 0), ("live", 1)] {
        let matched = NoteFolder::query(&ctx)
            .filter(ash_core::Filter::and([
                ash_core::Filter::eq("name", "inbox"),
                ash_core::Filter::related("notes", ash_core::Filter::eq("title", title)),
            ]))
            .count()
            .await
            .unwrap();
        assert_eq!(matched, expected, "related filter on `{title}`");
    }
}

async fn archival_resources_migrate_and_archive(db: TestDb) {
    let project = Project::for_db(&db);
    let migration =
        project.generate("create_archived_notes", &[&NoteFolder::DEF, &ArchivedNote::DEF]);
    let dialect = db.dialect().name();
    let up = std::fs::read_to_string(project.migrations().join(format!(
        "{}_create_archived_notes.{dialect}.up.sql",
        migration.version
    )))
    .unwrap();
    let column = if dialect == "postgres" {
        "\"archived_at\" TIMESTAMPTZ"
    } else {
        "\"archived_at\" TEXT"
    };
    assert!(up.contains(column), "{up}");

    db.migrate(&project.migrations()).await.unwrap();
    match &db.db {
        Db::Sqlite(sqlite) => archive_and_restore(Context::new(sqlite.clone())).await,
        Db::Postgres(pg) => archive_and_restore(Context::new(pg.clone())).await,
    }
    db.rollback(&project.migrations()).await.unwrap();
    assert!(db.schema().await.tables.is_empty());
}
on_every_backend!(archival_resources_migrate_and_archive);
