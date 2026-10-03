//! The desk's resources.

pub mod agent;
pub mod audit_event;
pub mod comment;
pub mod org;
pub mod tag;
pub mod ticket;
pub mod ticket_tag;

pub use agent::Agent;
pub use audit_event::AuditEvent;
pub use comment::Comment;
pub use org::Org;
pub use tag::Tag;
pub use ticket::Ticket;
pub use ticket_tag::TicketTag;
