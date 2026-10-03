//! Transformers rebuild `resource!` with a shared parser. These resources compile only
//! if the rebuilt call is valid, in each stacking order and with sections written the
//! ways `resource!` accepts.

use ash_archival::{archival, archive_def};
use ash_authentication::authentication;
use ash_core::{Context, Resource, ResourceExt};
use ash_memory::Memory;
use ash_state_machine::{StateMachineDef, state_machine};

/// Stands in for a third-party `extend` macro.
macro_rules! tag {
    ($name:ident, { label: $label:literal; }) => {
        impl $name {
            pub fn label() -> &'static str {
                $label
            }
        }
    };
}

pub mod ticket_mod {
    use super::*;
    use uuid::Uuid;

    // `table` and `extend` without `;`, before the sections a transformer edits.
    #[archival]
    #[state_machine]
    resource! {
        /// A support ticket.
        #[derive(Default)]
        Ticket {
            table "tickets"
            extend tag! { label: "support"; }

            attributes {
                id: Uuid [pk];
                title: String;
            }

            state_machine {
                state_attribute status;
                initial: "open";
                transition close, from: ["open"], to: "closed";
            }

            actions {
                create create { primary; accept [title]; },
                read read { primary; };
                update close {}
                destroy destroy { primary; }
            }
        }
    }
}
use ticket_mod::Ticket;

pub mod job_mod {
    use super::*;
    use uuid::Uuid;

    #[state_machine]
    #[archival]
    resource! {
        Job {
            table "jobs";

            attributes {
                id: Uuid [pk];
                // A path through a module named like the state attribute.
                kind: super::status::Kind;
            }

            state_machine {
                state_attribute status;
                initial: "queued";
                transition run, from: ["queued"], to: "running";
            }

            actions {
                create create { primary; accept [kind]; }
                read read { primary; }
                update run {}
                destroy destroy { primary; }
            }
        }
    }
}
use job_mod::Job;

pub mod status {
    pub type Kind = String;
}

pub mod member_mod {
    use super::*;
    use uuid::Uuid;

    #[authentication]
    #[archival]
    resource! {
        Member {
            table "members";

            attributes {
                id: Uuid [pk];
                email: String;
                name: Option<String>;
                hashed_password: Option<String>;
            }

            extensions [];

            actions {
                read read { primary; }
                create register_with_password { accept [email, name]; }
                destroy destroy { primary; }
            }
        }
    }
}
use member_mod::Member;

#[tokio::test]
async fn archival_then_state_machine() {
    assert_eq!(Ticket::label(), "support");
    assert!(Ticket::DEF.attribute("title").is_some());
    assert!(Ticket::DEF.attribute("status").is_some());
    assert!(Ticket::DEF.attribute("archived_at").is_some());
    assert!(Ticket::DEF.action("close").is_some());
    assert!(Ticket::DEF.action("destroy").unwrap().soft);
    assert!(Ticket::DEF.extension::<StateMachineDef>().is_some());
    assert!(archive_def::<Ticket>().is_some());
    let _: Ticket = Ticket::default();

    let ctx = Context::new(Memory::new());
    let ticket = Ticket::create(&ctx).title("Printer").await.unwrap();
    assert_eq!(ticket.status, "open");
    let closed = Ticket::close(&ctx, ticket.id).await.unwrap();
    assert_eq!(closed.status, "closed");
    closed.destroy(&ctx).await.unwrap();
    assert_eq!(Ticket::query(&ctx).count().await.unwrap(), 0);
}

#[tokio::test]
async fn state_machine_then_archival() {
    assert!(Job::DEF.attribute("status").is_some());
    assert!(Job::DEF.extension::<StateMachineDef>().is_some());
    assert!(archive_def::<Job>().is_some());

    let ctx = Context::new(Memory::new());
    let job = Job::create(&ctx).kind("export").await.unwrap();
    assert_eq!(Job::run(&ctx, job.id).await.unwrap().status, "running");
}

#[tokio::test]
async fn authentication_extends_a_register_action_it_finds() {
    let ctx = Context::new(Memory::new());
    let member = Member::register_with_password(&ctx)
        .email("ada@example.com")
        .name("Ada")
        .password("correct horse battery")
        .password_confirmation("correct horse battery")
        .await
        .unwrap();
    assert_eq!(member.name.as_deref(), Some("Ada"));
    assert!(member.hashed_password.unwrap().starts_with("$argon2id$"));
    assert!(archive_def::<Member>().is_some());
}
