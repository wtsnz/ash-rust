use ash_core::{Context, DataLayer, Resource, ResourceExt};

use crate::fixtures::archived_notes::ArchivedNote;
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
}

async fn archival_resources_migrate_and_archive(db: TestDb) {
    let project = Project::for_db(&db);
    let migration = project.generate("create_archived_notes", &[&ArchivedNote::DEF]);
    let dialect = db.dialect().name();
    let up = std::fs::read_to_string(project.migrations().join(format!(
        "{}_create_archived_notes.{dialect}.up.sql",
        migration.version
    )))
    .unwrap();
    let column = if dialect == "postgres" {
        "\"archived_at\" TIMESTAMPTZ\n"
    } else {
        "\"archived_at\" TEXT\n"
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
