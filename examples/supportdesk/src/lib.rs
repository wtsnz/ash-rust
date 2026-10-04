//! # Supportdesk
//!
//! A multi-tenant support desk, built here on ash-rust and in `examples/elixir/supportdesk`
//! on Elixir Ash, to benchmark real-world Ash features against each other. See the README
//! for the contract both apps implement.

pub mod fixture;
pub mod resources;
pub mod server;

use ash_core::domain;

pub use resources::{Agent, AuditEvent, Comment, Org, Tag, Ticket, TicketTag};

domain! {
    Desk {
        resources {
            Org;
            Agent;
            Tag;
            Ticket;
            Comment;
            TicketTag;
            AuditEvent;
        }
    }
}
