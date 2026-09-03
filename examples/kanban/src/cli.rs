use std::path::PathBuf;

use clap::{Parser, Subcommand};
use uuid::Uuid;

use crate::kanban::board::fields as b_f;
use crate::kanban::card::fields as c_f;
use crate::workspaces::workspace::fields as ws_f;
use crate::{
    Card, actor_user, board_template_multi, move_card_multi, open_sqlite_app,
    workspace_with_owner_multi,
};

#[derive(Parser)]
#[command(name = "kanban", about = "Trello-like Kanban on ash-rust (Workspaces + Kanban domains)")]
pub struct Cli {
    /// SQLite file. Created if missing.
    #[arg(long, env = "KANBAN_DB", default_value = "kanban.db", global = true)]
    pub db: PathBuf,

    /// Acting user id.
    #[arg(long = "as", env = "KANBAN_AS", global = true)]
    pub actor: Option<Uuid>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run in-memory end-to-end demo
    Demo,

    /// User operations (Workspaces domain)
    #[command(subcommand)]
    User(UserCommand),

    /// Workspace operations (Workspaces domain)
    #[command(subcommand)]
    Workspace(WorkspaceCommand),

    /// Board operations (Kanban domain)
    #[command(subcommand)]
    Board(BoardCommand),

    /// Card operations (Kanban domain)
    #[command(subcommand)]
    Card(CardCommand),
}

#[derive(Subcommand)]
pub enum UserCommand {
    /// Register a user
    Register {
        name: String,
        email: String,
    },
    /// List all registered users
    List,
}

#[derive(Subcommand)]
pub enum WorkspaceCommand {
    /// Create a workspace and enroll the owner as admin
    Create {
        name: String,
        slug: String,
        #[arg(long)]
        owner: Option<Uuid>,
    },
    /// List workspaces with member counts
    List,
}

#[derive(Subcommand)]
pub enum BoardCommand {
    /// Create a board
    Create {
        workspace_id: Uuid,
        name: String,
        #[arg(long)]
        description: Option<String>,
        /// Automatically initialize with To Do, In Progress, and Done lists
        #[arg(long)]
        template: bool,
    },
    /// List boards with list and card counts
    List {
        #[arg(long)]
        workspace_id: Option<Uuid>,
    },
}

#[derive(Subcommand)]
pub enum CardCommand {
    /// Create a card
    Create {
        board_id: Uuid,
        list_id: Uuid,
        title: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, default_value_t = 0)]
        position: i64,
    },
    /// List cards
    List {
        #[arg(long)]
        list_id: Option<Uuid>,
    },
    /// Atomically move card to another list
    Move {
        card_id: Uuid,
        to_list: Uuid,
        #[arg(long, default_value_t = 0)]
        position: i64,
        #[arg(long)]
        note: Option<String>,
    },
}

pub async fn run(cli: Cli) -> ash_core::Result<()> {
    match cli.command {
        Command::Demo => {
            crate::demo().await?;
        }
        Command::User(cmd) => {
            let (workspaces, _kanban) = open_sqlite_app(&cli.db).await?;
            match cmd {
                UserCommand::Register { name, email } => {
                    let user = workspaces.register_user(name, email).await?;
                    println!("Registered user: {} ({}) [id: {}]", user.name, user.email, user.id);
                }
                UserCommand::List => {
                    let users = workspaces.list_users().all().await?;
                    if users.is_empty() {
                        println!("No users registered.");
                    } else {
                        for u in users {
                            println!("- {} <{}> (id: {})", u.name, u.email, u.id);
                        }
                    }
                }
            }
        }
        Command::Workspace(cmd) => {
            let (workspaces, _kanban) = open_sqlite_app(&cli.db).await?;
            let actor_id = cli.actor.unwrap_or_else(Uuid::new_v4);
            let as_actor = workspaces.with_actor(actor_user(actor_id));

            match cmd {
                WorkspaceCommand::Create { name, slug, owner } => {
                    let owner_id = owner.or(cli.actor).unwrap_or(actor_id);
                    let multi = workspace_with_owner_multi(&as_actor.ctx, owner_id, &name, &slug)?;
                    let res = as_actor.run_multi(multi).await?;
                    let ws = res.get::<crate::Workspace>("workspace")?;
                    println!("Created workspace '{}' (slug: {}) [id: {}]", ws.name, ws.slug, ws.id);
                }
                WorkspaceCommand::List => {
                    let wss = as_actor
                        .workspaces()
                        .aggregate(ws_f::member_count)
                        .all()
                        .await?;
                    if wss.is_empty() {
                        println!("No workspaces found.");
                    } else {
                        for ws in wss {
                            println!(
                                "- {} (slug: {}) [id: {}] — members: {}",
                                ws.name,
                                ws.slug,
                                ws.id,
                                ws.member_count.unwrap_or(0)
                            );
                        }
                    }
                }
            }
        }
        Command::Board(cmd) => {
            let (_workspaces, kanban) = open_sqlite_app(&cli.db).await?;
            let actor_id = cli.actor.unwrap_or_else(Uuid::new_v4);
            let as_actor = kanban.with_actor(actor_user(actor_id));

            match cmd {
                BoardCommand::Create {
                    workspace_id,
                    name,
                    description,
                    template,
                } => {
                    if template {
                        let multi = board_template_multi(
                            &as_actor.ctx,
                            workspace_id,
                            &name,
                            description.as_deref(),
                        )?;
                        let res = as_actor.run_multi(multi).await?;
                        let board = res.get::<crate::Board>("board")?;
                        println!(
                            "Created template board '{}' with To Do, In Progress, Done lists [id: {}]",
                            board.name, board.id
                        );
                    } else {
                        let board = as_actor.create_board(workspace_id, name, description).await?;
                        println!("Created board '{}' [id: {}]", board.name, board.id);
                    }
                }
                BoardCommand::List { workspace_id } => {
                    let mut q = as_actor
                        .boards()
                        .aggregate(b_f::list_count)
                        .aggregate(b_f::card_count);
                    if let Some(ws_id) = workspace_id {
                        q = q.filter(b_f::workspace_id.eq(ws_id));
                    }
                    let boards = q.all().await?;
                    if boards.is_empty() {
                        println!("No boards found.");
                    } else {
                        for b in boards {
                            println!(
                                "- {} [id: {}] — lists: {}, cards: {}",
                                b.name,
                                b.id,
                                b.list_count.unwrap_or(0),
                                b.card_count.unwrap_or(0)
                            );
                        }
                    }
                }
            }
        }
        Command::Card(cmd) => {
            let (_workspaces, kanban) = open_sqlite_app(&cli.db).await?;
            let actor_id = cli.actor.unwrap_or_else(Uuid::new_v4);
            let as_actor = kanban.with_actor(actor_user(actor_id));

            match cmd {
                CardCommand::Create {
                    board_id,
                    list_id,
                    title,
                    description,
                    position,
                } => {
                    let card = as_actor
                        .create_card(board_id, list_id, title, description, position)
                        .await?;
                    println!("Created card '{}' [id: {}] at position {}", card.title, card.id, card.position);
                }
                CardCommand::List { list_id } => {
                    let mut q = as_actor
                        .cards()
                        .aggregate(c_f::checklist_count)
                        .aggregate(c_f::comment_count);
                    if let Some(lid) = list_id {
                        q = q.filter(c_f::list_id.eq(lid));
                    }
                    let cards = q.all().await?;
                    if cards.is_empty() {
                        println!("No cards found.");
                    } else {
                        for c in cards {
                            println!(
                                "- {} [id: {}] (list: {}) — checklist items: {}, comments: {}",
                                c.title,
                                c.id,
                                c.list_id,
                                c.checklist_count.unwrap_or(0),
                                c.comment_count.unwrap_or(0)
                            );
                        }
                    }
                }
                CardCommand::Move {
                    card_id,
                    to_list,
                    position,
                    note,
                } => {
                    let card = as_actor.get_card(card_id).await?;
                    let multi = move_card_multi(&as_actor.ctx, card, to_list, position, note)?;
                    let res = as_actor.run_multi(multi).await?;
                    let moved = res.get::<Card>("moved_card")?;
                    println!("Moved card '{}' to list {} at position {}", moved.title, moved.list_id, moved.position);
                }
            }
        }
    }
    Ok(())
}
