use std::path::PathBuf;

use ash_core::Context;
use ash_sqlite::Sqlite;
use clap::{Parser, Subcommand, ValueEnum};
use uuid::Uuid;

use crate::representative::fields as rep;
use crate::ticket::fields as t;
use crate::ticket::{actor_customer, actor_representative};
use crate::{Representative, Ticket, open_sqlite};

#[derive(Parser)]
#[command(name = "helpdesk", about = "File-backed helpdesk on ash-core + SQLite")]
pub struct Cli {
    /// SQLite file. Created if missing. HELPDESK_DB overrides the default.
    #[arg(
        long,
        env = "HELPDESK_DB",
        default_value = "helpdesk.db",
        global = true
    )]
    pub db: PathBuf,

    /// Actor id for ticket commands. HELPDESK_AS also works.
    #[arg(long = "as", env = "HELPDESK_AS", global = true)]
    pub actor: Option<Uuid>,

    /// Skip auto-detect (uuid in representatives → representative, else customer).
    #[arg(long, env = "HELPDESK_ROLE", global = true)]
    pub role: Option<Role>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Role {
    Customer,
    Representative,
}

#[derive(Subcommand)]
pub enum Command {
    /// In-memory story. Does not touch the database file.
    Demo,
    /// Print a new uuid to use as a customer `--as`.
    NewId,
    #[command(subcommand)]
    Rep(RepCommand),
    #[command(subcommand)]
    Ticket(TicketCommand),
}

#[derive(Subcommand)]
pub enum RepCommand {
    /// Hire a representative. Prints their id.
    Add {
        name: String,
    },
    List,
}

#[derive(Subcommand)]
pub enum TicketCommand {
    Open {
        subject: Vec<String>,
    },
    List {
        /// Only open tickets.
        #[arg(long)]
        open: bool,
        /// Load the assigned representative.
        #[arg(long)]
        with_rep: bool,
        /// Include subject_length.
        #[arg(long)]
        calc: bool,
    },
    Show {
        id: Uuid,
    },
    Assign {
        id: Uuid,
        representative: Uuid,
    },
    Close {
        id: Uuid,
    },
}

pub async fn run(cli: Cli) -> ash_core::Result<()> {
    match cli.command {
        Command::Demo => crate::demo().await,
        Command::NewId => {
            println!("{}", Uuid::new_v4());
            Ok(())
        }
        Command::Rep(cmd) => {
            let ctx = open_sqlite(&cli.db).await?;
            run_rep(&ctx, cmd).await
        }
        Command::Ticket(cmd) => {
            let ctx = open_sqlite(&cli.db).await?;
            let actor_id = cli.actor.ok_or_else(|| {
                ash_core::Error::Invalid(
                    "ticket commands need --as <uuid> (try `helpdesk new-id`)".into(),
                )
            })?;
            let ctx = with_actor(&ctx, actor_id, cli.role).await?;
            run_ticket(&ctx, cmd).await
        }
    }
}

async fn with_actor(
    ctx: &Context<Sqlite>,
    id: Uuid,
    role: Option<Role>,
) -> ash_core::Result<Context<Sqlite>> {
    let actor = match role {
        Some(Role::Customer) => actor_customer(id),
        Some(Role::Representative) => actor_representative(id),
        None => match ash_core::get::<Representative, _>(ctx, id).await {
            Ok(_) => actor_representative(id),
            Err(ash_core::Error::NotFound) => actor_customer(id),
            Err(err) => return Err(err),
        },
    };
    Ok(ctx.with_actor(actor))
}

async fn run_rep(ctx: &Context<Sqlite>, cmd: RepCommand) -> ash_core::Result<()> {
    match cmd {
        RepCommand::Add { name } => {
            let rep = Representative::create(ctx).name(name).await?;
            println!("representative  {}  {}", rep.id, rep.name);
        }
        RepCommand::List => {
            let reps = Representative::query(ctx).sort(rep::name).load().await?;
            if reps.is_empty() {
                println!("(none)");
            }
            for rep in reps {
                println!("representative  {}  {}", rep.id, rep.name);
            }
        }
    }
    Ok(())
}

async fn run_ticket(ctx: &Context<Sqlite>, cmd: TicketCommand) -> ash_core::Result<()> {
    match cmd {
        TicketCommand::Open { subject } => {
            let subject = subject.join(" ");
            if subject.is_empty() {
                return Err(ash_core::Error::Invalid("subject is required".into()));
            }
            let ticket = Ticket::open(ctx).subject(subject).await?;
            print_ticket(&ticket);
        }
        TicketCommand::List {
            open,
            with_rep,
            calc,
        } => {
            let mut query = Ticket::query(ctx);
            if open {
                query = query.filter(t::status.eq("open"));
            }
            if with_rep {
                query = query.include(t::representative);
            }
            if calc {
                query = query.calc(t::subject_length);
            }
            let tickets = query.sort(t::subject).load().await?;
            if tickets.is_empty() {
                println!("(none)");
            }
            for ticket in tickets {
                print_ticket(&ticket);
            }
        }
        TicketCommand::Show { id } => {
            let ticket = Ticket::query(ctx)
                .filter(t::id.eq(id))
                .calc(t::subject_length)
                .include(t::representative)
                .one()
                .await?;
            print_ticket_verbose(&ticket);
        }
        TicketCommand::Assign { id, representative } => {
            let ticket = Ticket::assign(ctx, id).representative_id(representative).await?;
            print_ticket(&ticket);
        }
        TicketCommand::Close { id } => {
            let ticket = Ticket::close(ctx, id).await?;
            print_ticket(&ticket);
        }
    }
    Ok(())
}

fn print_ticket(ticket: &Ticket) {
    let assignee = ticket
        .representative_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "-".into());
    let extra = match ticket.representative.as_option() {
        Ok(Some(rep)) => format!("  assignee_name={}", rep.name),
        Ok(None) => "  assignee_name=-".into(),
        Err(_) => String::new(),
    };
    let length = ticket
        .subject_length
        .map(|n| format!("  length={n}"))
        .unwrap_or_default();
    println!(
        "ticket  {}  {:<6}  {}  opener={}  assignee={}{extra}{length}",
        ticket.id,
        ticket.status.as_str(),
        ticket.subject,
        ticket.opener_id,
        assignee,
    );
}

fn print_ticket_verbose(ticket: &Ticket) {
    println!("ticket       {}", ticket.id);
    println!("status       {}", ticket.status.as_str());
    println!("subject      {}", ticket.subject);
    println!("opener       {}", ticket.opener_id);
    match ticket.representative.as_option() {
        Ok(Some(rep)) => {
            println!("assignee     {}  {}", rep.id, rep.name);
        }
        Ok(None) => println!("assignee     -"),
        Err(_) => match ticket.representative_id {
            Some(id) => println!("assignee     {id}"),
            None => println!("assignee     -"),
        },
    }
    if let Some(length) = ticket.subject_length {
        println!("length       {length}");
    }
}
