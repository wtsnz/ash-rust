# ash-archival

Soft delete for **ash-rust** resources, following Elixir's [`ash_archival`](https://hexdocs.pm/ash_archival). Destroying a record sets `archived_at` instead of deleting it, and reads hide archived records.

```rust
use ash_archival::archival;
use ash_core::resource;
use uuid::Uuid;

#[archival]
resource! {
    Post {
        table "posts";

        attributes {
            id: Uuid [pk];
            title: String;
        }

        relationships {
            has_many comments: Comment [fk: post_id];
        }

        archive {
            exclude_read_actions [archived];
            exclude_destroy_actions [purge];
            archive_related [comments];
            unarchive_action unarchive;
        }

        actions {
            create create { primary; accept [title]; }
            read read { primary; }
            read archived { prepare filter(!archived_at.is_nil()); }
            destroy destroy { primary; }
            destroy purge;
        }
    }
}
```

## What `#[archival]` does

1. Adds `archived_at: Option<UtcDateTime>` unless the resource declares it. Migrations pick it up like any attribute (`TIMESTAMPTZ` on Postgres, `TEXT` on SQLite).
2. Adds `prepare filter(archived_at.is_nil())` to every read action except `exclude_read_actions`. `Post::query(&ctx)` and relationship loads use the primary read, so they skip archived records. Aggregates still count them.
3. Makes every destroy action except `exclude_destroy_actions` a soft destroy that sets `archived_at`. Like an update, it raises the optimistic lock version and `updated_at`, and it writes only the columns it changes. The row stays, so `on_delete` cascades do not run.
4. After archiving the record, destroys its `archive_related` relationships with their primary destroy action, which archives them when they use `#[archival]` too. Archived children are hidden from that lookup, so a cycle in the data ends.
5. With `unarchive_action`, adds (or extends) an update action that clears `archived_at`.
6. Registers `ArchiveDef` in the resource's extensions and adds `post.is_archived()`.

## Options

| Option | Default | Meaning |
| :--- | :--- | :--- |
| `attribute` | `archived_at` | The attribute that holds the archive time. |
| `exclude_read_actions` | `[]` | Read actions that still return archived records. |
| `exclude_destroy_actions` | `[]` | Destroy actions that really delete. |
| `archive_related` | `[]` | `has_many` / `has_one` relationships to archive along with the record. |
| `unarchive_action` | none | An update action that clears the attribute. |

## Hard deletes

An excluded destroy action (`purge` above) really deletes. Its cascades, both `on_delete: cascade` and `cascade_destroy`, delete the children too, archived or not, even when the children use `#[archival]`: archiving them would leave rows pointing at a deleted parent. A child with only soft destroy actions has its row deleted directly, as `ON DELETE CASCADE` would.

## Unarchiving

Primary reads hide archived records, so updating or destroying one by id, through Rust, bulk actions or GraphQL, reports it as not found. `ash_archival::unarchive::<Post, _>(&ctx, id)` loads the record with the first of `exclude_read_actions`, which keeps read policies and tenancy, then runs `unarchive_action`.

## Unique identities

As in AshArchival, identities still cover archived rows. To let a new record reuse an archived record's key, scope the identity:

```rust
identities {
    identity unique_slug: [slug], where: "archived_at IS NULL";
}
```
