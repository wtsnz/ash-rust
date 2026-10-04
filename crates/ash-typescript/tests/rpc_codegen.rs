//! The AshTypescript client generated for actions of every shape (offset, keyset and
//! required pages, gets that may find nothing, destroys with metadata, named identities,
//! generic actions returning maps), type-checked with `tsc` where it's installed.

use std::process::Command;

use ash_core::{ChangeContext, FieldMap, resource};
use ash_memory::Memory;
use ash_typescript::rpc::{ClientConfig, Identity, Rpc};
use uuid::Uuid;

resource! {
    Book {
        table "codegen_books";

        attributes {
            id: Uuid [pk];
            isbn: String;
            title: String;
            pages: Option<i64>;
        }

        identities {
            identity unique_isbn: [isbn];
        }

        actions {
            read read { primary; pagination offset: true, countable: true, required: false; }
            read paged { pagination keyset: true, offset: true, countable: by_default, required: true, default_limit: 10; }
            read all {}
            create create { primary; accept [isbn, title, pages]; }
            update retitle { accept [title]; }
            destroy destroy {
                primary;
                metadata gone_at: String;
                change func(note_gone);
            }
            generic stats {
                argument since: Option<String>;
                returns FieldMap;
                run |input| async move {
                    let _ = input.since;
                    Ok(FieldMap::new())
                };
            }
        }
    }
}

fn note_gone(ctx: &mut ChangeContext<'_>) -> ash_core::Result<()> {
    ctx.after_action(|record| {
        ash_core::put_metadata(record, "gone_at", "now");
        Ok(())
    });
    Ok(())
}

fn rpc() -> Rpc<Memory> {
    Rpc::new()
        .action::<Book>("list_books", "read")
        .action::<Book>("paged_books", "paged")
        .action::<Book>("all_books", "all")
        .action_with::<Book>("find_book", "read", |o| o.get_by(&["isbn"]).not_found_error(false))
        .action::<Book>("create_book", "create")
        .action_with::<Book>("retitle_book", "retitle", |o| o.identities(vec![Identity::PrimaryKey, Identity::Named("unique_isbn")]))
        .action::<Book>("destroy_book", "destroy")
        .action::<Book>("book_stats", "stats")
        .typed_query::<Book>("book_titles", "read", serde_json::json!(["id", "title"]))
}

#[test]
fn every_shape_generates_a_client_that_type_checks() {
    let config = ClientConfig { validation_functions: true, channel_functions: true, ..ClientConfig::default() };
    let client = rpc().typescript_client(&config);
    // Each shape, as AshTypescript writes it.
    assert!(client.rpc.contains("type: \"offset\";"), "offset pages");
    assert!(client.rpc.contains("export type PagedBooksResult<Fields extends PagedBooksFields> ="), "required pages");
    assert!(client.rpc.contains("> = InferResult<BookResourceSchema, Fields> | null;"), "a get that may find nothing");
    assert!(client.rpc.contains("identity: UUID | { isbn: string };"), "named identities");
    assert!(client.rpc.contains("export type DestroyBookMetadata = {"), "a destroy's metadata");
    assert!(client.rpc.contains("export type InferBookStatsResult = Record<string, any>;"), "a generic action's map");
    assert!(client.rpc.contains("export const bookTitles = [\"id\", \"title\"] satisfies ListBooksFields;"), "a typed query");

    let dir = std::env::temp_dir().join(format!("ash_ts_codegen_{}", Uuid::new_v4().simple()));
    client.write(&dir).unwrap();
    // A stand-in for phoenix's Channel, so it checks without node_modules.
    std::fs::create_dir_all(dir.join("node_modules/phoenix")).unwrap();
    std::fs::write(dir.join("node_modules/phoenix/index.d.ts"), "export declare class Channel {}\n").unwrap();
    let Ok(output) = Command::new("tsc")
        .args(["--noEmit", "--strict", "--target", "ES2022", "--moduleResolution", "node", "--lib", "ES2022,DOM", "--skipLibCheck"])
        .arg(dir.join("ash_rpc.ts"))
        .current_dir(&dir)
        .output()
    else {
        eprintln!("tsc not installed; skipping the type check");
        return;
    };
    assert!(
        output.status.success(),
        "tsc failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
