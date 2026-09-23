use ash_core::{Resource, ResourceDef};

pub fn new_code() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub mod org {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Org {
            table "orgs";
            attributes {
                id: Uuid [pk];
                name: String;
            }
        }
    }
}

pub mod base {
    pub use super::org::Org;
    use ash_core::{domain, resource};
    use uuid::Uuid;

    resource! {
        Ticket {
            table "tickets";
            attributes {
                id: Uuid [pk];
                subject: String;
                status: String = "open";
                notes: Option<String>;
                org_id: Uuid;
            }
            relationships {
                belongs_to org: Org [fk: org_id, on_delete: cascade];
            }
            identities {
                identity unique_subject: [subject];
            }
            actions {
                read read { primary; }
            }
        }
    }

    domain! {
        Helpdesk {
            resources {
                Org;
                Ticket;
            }
        }
    }
}

pub mod without_identity {
    use super::base::Org;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Ticket {
            table "tickets";
            attributes {
                id: Uuid [pk];
                subject: String;
                status: String = "open";
                notes: Option<String>;
                org_id: Uuid;
            }
            relationships {
                belongs_to org: Org [fk: org_id, on_delete: cascade];
            }
        }
    }
}

pub mod unlinked {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Ticket {
            table "tickets";
            attributes {
                id: Uuid [pk];
                subject: String;
                status: String = "open";
                notes: Option<String>;
                org_id: Uuid;
            }
            identities {
                identity unique_subject: [subject];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod with_priority {
    use super::base::Org;
    use ash_core::{domain, resource};
    use uuid::Uuid;

    resource! {
        Ticket {
            table "tickets";
            attributes {
                id: Uuid [pk];
                subject: String;
                status: String = "open";
                notes: Option<String>;
                org_id: Uuid;
                priority: i64 = 3;
            }
            relationships {
                belongs_to org: Org [fk: org_id, on_delete: cascade];
            }
            identities {
                identity unique_subject: [subject];
            }
            actions {
                read read { primary; }
            }
        }
    }

    domain! {
        Helpdesk {
            resources {
                Org;
                Ticket;
            }
        }
    }
}

pub mod with_priority_and_category {
    use super::base::Org;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Ticket {
            table "tickets";
            attributes {
                id: Uuid [pk];
                subject: String;
                status: String = "open";
                notes: Option<String>;
                org_id: Uuid;
                priority: i64 = 3;
                category: Option<String>;
            }
            relationships {
                belongs_to org: Org [fk: org_id, on_delete: cascade];
            }
            identities {
                identity unique_subject: [subject];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod with_runtime_default {
    use super::base::Org;
    use super::new_code;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Ticket {
            table "tickets";
            attributes {
                id: Uuid [pk];
                subject: String;
                status: String = "open";
                notes: Option<String>;
                org_id: Uuid;
                code: String [default_fn: new_code];
            }
            relationships {
                belongs_to org: Org [fk: org_id, on_delete: cascade];
            }
            identities {
                identity unique_subject: [subject];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod without_notes {
    use super::base::Org;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Ticket {
            table "tickets";
            attributes {
                id: Uuid [pk];
                subject: String;
                status: String = "new";
                org_id: Uuid;
            }
            relationships {
                belongs_to org: Org [fk: org_id, on_delete: cascade];
            }
            identities {
                identity unique_subject: [subject];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod renamed_subject {
    use super::base::Org;
    use ash_core::{domain, resource};
    use uuid::Uuid;

    resource! {
        Ticket {
            table "tickets";
            attributes {
                id: Uuid [pk];
                title: String;
                status: String = "open";
                notes: Option<String>;
                org_id: Uuid;
            }
            relationships {
                belongs_to org: Org [fk: org_id, on_delete: cascade];
            }
            identities {
                identity unique_subject: [title];
            }
            actions {
                read read { primary; }
            }
        }
    }

    domain! {
        Helpdesk {
            resources {
                Org;
                Ticket;
            }
        }
    }
}

pub mod required_notes {
    use super::base::Org;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Ticket {
            table "tickets";
            attributes {
                id: Uuid [pk];
                subject: String;
                status: String = "open";
                notes: String = "none";
                org_id: Uuid;
            }
            relationships {
                belongs_to org: Org [fk: org_id, on_delete: cascade];
            }
            identities {
                identity unique_subject: [subject];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod org_with_default_name {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Org {
            table "orgs";
            attributes {
                id: Uuid [pk];
                name: String = "unnamed";
            }
        }
    }
}

pub mod counter_text {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Counter {
            table "counters";
            attributes {
                id: Uuid [pk];
                value: String;
            }
        }
    }
}

pub mod counter_integer {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Counter {
            table "counters";
            attributes {
                id: Uuid [pk];
                value: i64;
            }
        }
    }
}

pub mod dev_renamed_estimate {
    use super::base::Org;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Ticket {
            table "tickets";
            attributes {
                id: Uuid [pk];
                title: String;
                status: String = "open";
                notes: Option<String>;
                org_id: Uuid;
                priority: i64 = 3;
                estimate: Option<String>;
            }
            relationships {
                belongs_to org: Org [fk: org_id, on_delete: cascade];
            }
            identities {
                identity unique_subject: [title];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod dev_final_ticket {
    use super::base::Org;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Ticket {
            table "tickets";
            attributes {
                id: Uuid [pk];
                title: String;
                status: String = "new";
                notes: Option<String>;
                org_id: Uuid;
                priority: i64 = 3;
                estimate: Option<i64>;
            }
            relationships {
                belongs_to org: Org [fk: org_id, on_delete: cascade];
            }
            identities {
                identity unique_subject: [title];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod dev_comment {
    use super::dev_final_ticket::Ticket;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Comment {
            table "comments";
            attributes {
                id: Uuid [pk];
                body: String;
                ticket_id: Uuid;
            }
            relationships {
                belongs_to ticket: Ticket [fk: ticket_id, on_delete: cascade];
            }
        }
    }
}

pub mod labeled_notes {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        LabeledNote {
            table "labeled_notes";
            attributes {
                id: Uuid [pk];
                title: String;
                status: String;
            }
            identities {
                identity unique_title: [title];
            }
            indexes {
                index by_status: [status];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod invoice_text {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Invoice {
            table "invoices";
            attributes {
                id: Uuid [pk];
                opened_at: String;
                amount: String;
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod invoice {
    use ash_core::{resource, Decimal, UtcDateTime};
    use uuid::Uuid;

    resource! {
        Invoice {
            table "invoices";
            attributes {
                id: Uuid [pk];
                opened_at: UtcDateTime;
                amount: Decimal = "12.50";
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub fn helpdesk() -> [&'static ResourceDef; 2] {
    [&base::Org::DEF, &base::Ticket::DEF]
}

pub fn helpdesk_with(ticket: &'static ResourceDef) -> [&'static ResourceDef; 2] {
    [&base::Org::DEF, ticket]
}

pub const ORG_ID: &str = "00000000-0000-0000-0000-00000000000a";
pub const OTHER_ORG_ID: &str = "00000000-0000-0000-0000-00000000000b";
pub const TICKET_ID: &str = "00000000-0000-0000-0000-000000000001";
