pub mod cli;
pub mod kanban;
pub mod workspaces;

use ash_core::{Actor, Multi, Result, domain};
use ash_sqlite::Sqlite;
use std::path::Path;
use uuid::Uuid;

pub use kanban::{
    BOARD_DEF, Board, CARD_DEF, CHECKLIST_ITEM_DEF, COMMENT_DEF, Card, ChecklistItem, Comment,
    LIST_DEF, List, board, card, checklist_item, comment, list,
};
pub use workspaces::{
    USER_DEF, User, WORKSPACE_DEF, WORKSPACE_MEMBER_DEF, Workspace, WorkspaceMember, member, user,
    workspace,
};

domain! {
    Workspaces {
        resources {
            User {
                define register_user, action: register, args: [name: String, email: String];
                define get_user, action: read, get_by: id;
                define list_users, action: read;
            };
            Workspace {
                define create_workspace, action: create, args: [name: String, slug: String];
                define get_workspace, action: read, get_by: id;
                define list_workspaces, action: read;
            };
            WorkspaceMember {
                define add_member, action: add, args: [workspace_id: ::uuid::Uuid, user_id: ::uuid::Uuid, role: String];
                define remove_member, action: destroy, on: record;
                define list_members, action: read;
            };
        }
    }
}

domain! {
    Kanban {
        resources {
            Board {
                define create_board, action: create, args: [workspace_id: ::uuid::Uuid, name: String, description: Option<String>];
                define get_board, action: read, get_by: id;
                define list_boards, action: read;
                define archive_board, action: archive, on: record;
            };
            List {
                define create_list, action: create, args: [board_id: ::uuid::Uuid, title: String, position: i64];
                define get_list, action: read, get_by: id;
                define list_lists, action: read;
                define rename_list, action: rename, on: record, args: [title: String];
                define move_list, action: move_position, on: record, args: [position: i64];
                define archive_list, action: archive, on: record;
            };
            Card {
                define create_card, action: create, args: [board_id: ::uuid::Uuid, list_id: ::uuid::Uuid, title: String, description: Option<String>, position: i64];
                define get_card, action: read, get_by: id;
                define list_cards, action: read;
                define move_card, action: move_to_list, on: record, args: [list_id: ::uuid::Uuid, position: i64];
                define assign_card, action: assign, on: record, args: [assignee_id: Option<::uuid::Uuid>];
                define archive_card, action: archive, on: record;
            };
            ChecklistItem {
                define add_checklist_item, action: create, args: [card_id: ::uuid::Uuid, title: String, position: i64];
                define toggle_checklist_item, action: toggle, on: record, args: [completed: bool];
                define delete_checklist_item, action: destroy, on: record;
                define list_checklist_items, action: read;
            };
            Comment {
                define add_comment, action: create, args: [card_id: ::uuid::Uuid, body: String];
                define update_comment, action: update_body, on: record, args: [body: String];
                define delete_comment, action: destroy, on: record;
                define list_comments, action: read;
            };
        }
    }
}

pub fn actor_user(id: Uuid) -> Actor {
    Actor::new(id).with_role("user")
}

pub fn actor_admin(id: Uuid) -> Actor {
    Actor::new(id).with_role("admin")
}

/// Helper that builds an atomic Ash.Multi pipeline to create a standard Kanban board:
/// Board -> "To Do" list -> "In Progress" list -> "Done" list -> Welcome Card with Checklist.
pub fn board_template_multi<D: ash_core::DataLayer + 'static>(
    ctx: &ash_core::Context<D>,
    workspace_id: Uuid,
    board_name: &str,
    description: Option<&str>,
) -> Result<Multi<D>> {
    let board_cs = Board::create(ctx)
        .workspace_id(workspace_id)
        .name(board_name)
        .description(description.map(|s| s.to_string()))
        .changeset()?;

    let multi = Multi::new()
        .create("board", board_cs)
        .create_from("todo_list", move |ctx, results| {
            let board = results.get::<Board>("board")?;
            List::create(ctx)
                .board_id(board.id)
                .title("To Do")
                .position(0_i64)
                .changeset()
        })
        .create_from("doing_list", move |ctx, results| {
            let board = results.get::<Board>("board")?;
            List::create(ctx)
                .board_id(board.id)
                .title("In Progress")
                .position(1_i64)
                .changeset()
        })
        .create_from("done_list", move |ctx, results| {
            let board = results.get::<Board>("board")?;
            List::create(ctx)
                .board_id(board.id)
                .title("Done")
                .position(2_i64)
                .changeset()
        })
        .create_from("welcome_card", move |ctx, results| {
            let board = results.get::<Board>("board")?;
            let todo_list = results.get::<List>("todo_list")?;
            Card::create(ctx)
                .board_id(board.id)
                .list_id(todo_list.id)
                .title("Welcome to your board!")
                .description(Some("Explore lists, add cards, and track subtasks.".into()))
                .position(0_i64)
                .changeset()
        })
        .create_from("task_1", move |ctx, results| {
            let card = results.get::<Card>("welcome_card")?;
            ChecklistItem::create(ctx)
                .card_id(card.id)
                .title("Create your first custom card")
                .position(0_i64)
                .changeset()
        })
        .create_from("task_2", move |ctx, results| {
            let card = results.get::<Card>("welcome_card")?;
            ChecklistItem::create(ctx)
                .card_id(card.id)
                .title("Invite team members to workspace")
                .position(1_i64)
                .changeset()
        });

    Ok(multi)
}

/// Helper that builds an atomic Ash.Multi pipeline to create a Workspace and enroll the owner as Admin.
pub fn workspace_with_owner_multi<D: ash_core::DataLayer + 'static>(
    ctx: &ash_core::Context<D>,
    owner_id: Uuid,
    name: &str,
    slug: &str,
) -> Result<Multi<D>> {
    let ws_cs = Workspace::create(ctx)
        .name(name)
        .slug(slug)
        .changeset()?;

    let multi = Multi::new()
        .create("workspace", ws_cs)
        .create_from("membership", move |ctx, results| {
            let ws = results.get::<Workspace>("workspace")?;
            WorkspaceMember::add(ctx)
                .workspace_id(ws.id)
                .user_id(owner_id)
                .role("admin")
                .changeset()
        });

    Ok(multi)
}

/// Helper that builds an atomic Ash.Multi pipeline to move a card to another list
/// and optionally add an activity comment explaining the move.
pub fn move_card_multi<D: ash_core::DataLayer + 'static>(
    ctx: &ash_core::Context<D>,
    card: Card,
    destination_list_id: Uuid,
    new_position: i64,
    note: Option<String>,
) -> Result<Multi<D>> {
    let card_id = card.id;
    let card_cs = card
        .move_to_list_on(ctx)
        .list_id(destination_list_id)
        .position(new_position)
        .changeset()?;

    let mut multi = Multi::new().update("moved_card", card_cs);

    if let Some(body) = note {
        multi = multi.create_from("movement_comment", move |ctx, _results| {
            Comment::create(ctx)
                .card_id(card_id)
                .body(body)
                .changeset()
        });
    }

    Ok(multi)
}

/// Helper that builds an atomic Ash.Multi pipeline to add multiple checklist items to a card.
pub fn add_checklist_items_multi<D: ash_core::DataLayer + 'static>(
    ctx: &ash_core::Context<D>,
    card_id: Uuid,
    items: Vec<String>,
) -> Result<Multi<D>> {
    let mut multi = Multi::new();
    for (idx, title) in items.into_iter().enumerate() {
        let step_name = format!("item_{idx}");
        let cs = ChecklistItem::create(ctx)
            .card_id(card_id)
            .title(title)
            .position(idx as i64)
            .changeset()?;
        multi = multi.create(step_name, cs);
    }
    Ok(multi)
}

pub async fn open_sqlite_app(
    path: impl AsRef<Path>,
) -> Result<(Workspaces<Sqlite>, Kanban<Sqlite>)> {
    if let Some(parent) = path.as_ref().parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|err| ash_core::Error::DataLayer(format!("create database directory: {err}")))?;
    }
    let db = Sqlite::file(path).await?;
    let workspaces = Workspaces::new(db.clone());
    let kanban = Kanban::new(db);

    workspaces.install().await?;
    kanban.install().await?;

    Ok((workspaces, kanban))
}

pub async fn demo() -> Result<()> {
    use ash_memory::Memory;

    println!("=== Kanban / Trello Example on ash-rust ===");

    let mem = Memory::new();
    let workspaces = Workspaces::new(mem.clone());
    let kanban = Kanban::new(mem);

    // 1. Register users
    let alice = workspaces.register_user("Alice", "alice@example.com").await?;
    let bob = workspaces.register_user("Bob", "bob@example.com").await?;
    println!("Registered users: {} ({}), {} ({})", alice.name, alice.email, bob.name, bob.email);

    // 2. Create Workspace with Alice as owner and Admin member via Multi
    let as_alice = workspaces.with_actor(actor_user(alice.id));
    let ws_multi = workspace_with_owner_multi(&as_alice.ctx, alice.id, "Acme Engineering", "acme-eng")?;
    let ws_results = as_alice.run_multi(ws_multi).await?;
    let workspace = ws_results.get::<Workspace>("workspace")?;
    println!("Created workspace: {} (slug: {})", workspace.name, workspace.slug);

    // Add Bob as a Member
    let member_cs = WorkspaceMember::add(&as_alice.ctx)
        .workspace_id(workspace.id)
        .user_id(bob.id)
        .role("member")
        .changeset()?;
    let _ = member_cs.commit(&as_alice.ctx).await?;
    println!("Added Bob as workspace member");

    // 3. Create a Board with default lists and welcome card via atomic Multi pipeline
    let as_alice_kanban = kanban.with_actor(actor_user(alice.id));
    let board_multi = board_template_multi(
        &as_alice_kanban.ctx,
        workspace.id,
        "Product Launch Q4",
        Some("Roadmap and tasks for the launch"),
    )?;
    let board_results = as_alice_kanban.run_multi(board_multi).await?;
    let board = board_results.get::<Board>("board")?;
    let welcome_card = board_results.get::<Card>("welcome_card")?;
    println!("Created board: {} with welcome card: '{}'", board.name, welcome_card.title);

    // 4. Inspect board with aggregates
    let loaded_board = as_alice_kanban
        .boards()
        .aggregate(kanban::board::fields::list_count)
        .aggregate(kanban::board::fields::card_count)
        .one()
        .await?;

    println!(
        "Board stats -> Lists: {:?}, Cards: {:?}",
        loaded_board.list_count, loaded_board.card_count
    );

    println!("Kanban demo completed successfully!");
    Ok(())
}
