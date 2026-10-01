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
    use ash_core::{Decimal, UtcDateTime, resource};
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

pub mod renamed_table {
    pub use super::base::Org;
    use ash_core::{domain, resource};
    use uuid::Uuid;

    resource! {
        Issue {
            table "issues";
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
                Issue;
            }
        }
    }
}

/// `tickets` renamed to `issues`, with `subject` renamed to `title`, a new default (which
/// rebuilds the table on SQLite), and `on_delete: restrict`.
pub mod renamed_table_changed {
    pub use super::base::Org;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Issue {
            table "issues";
            attributes {
                id: Uuid [pk];
                title: String;
                status: String = "closed";
                notes: Option<String>;
                org_id: Uuid;
            }
            relationships {
                belongs_to org: Org [fk: org_id, on_delete: restrict];
            }
            identities {
                identity unique_title: [title];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub const ORG_ID: &str = "00000000-0000-0000-0000-00000000000a";
pub const OTHER_ORG_ID: &str = "00000000-0000-0000-0000-00000000000b";
pub const TICKET_ID: &str = "00000000-0000-0000-0000-000000000001";

pub mod bounded_notes_plain {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        BoundedNote {
            table "bounded_notes";
            attributes {
                id: Uuid [pk];
                title: String;
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod bounded_notes {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        BoundedNote {
            table "bounded_notes";
            attributes {
                id: Uuid [pk];
                title: String;
            }
            checks {
                check titled: "title <> ''";
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod file_blob {
    use ash_core::{Binary, resource};
    use uuid::Uuid;

    resource! {
        FileBlob {
            table "file_blobs";
            attributes {
                id: Uuid [pk];
                payload: Binary = "aGVsbG8=";
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod deadline {
    use ash_core::{Date, resource};
    use uuid::Uuid;

    resource! {
        Deadline {
            table "deadlines";
            attributes {
                id: Uuid [pk];
                due_on: Date = "2024-02-29";
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod gauge {
    use ash_core::{Float, resource};
    use uuid::Uuid;

    resource! {
        Gauge {
            table "gauges";
            attributes {
                id: Uuid [pk];
                weight: Float = "1.50";
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod indexed_notes_v1 {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        IndexedNote {
            table "indexed_notes";
            attributes {
                id: Uuid [pk];
                title: String;
            }
            statements {
                statement by_title {
                    up "CREATE INDEX IF NOT EXISTS indexed_notes_by_title ON indexed_notes (title)";
                    down "DROP INDEX IF EXISTS indexed_notes_by_title";
                }
                statement by_id {
                    up "CREATE INDEX IF NOT EXISTS indexed_notes_by_id ON indexed_notes (id)";
                    down "DROP INDEX IF EXISTS indexed_notes_by_id";
                }
            }
            actions {
                read read { primary; }
            }
        }
    }
}

/// `by_title` changes, and `title` becomes optional, which rebuilds the table on SQLite.
pub mod indexed_notes_v2 {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        IndexedNote {
            table "indexed_notes";
            attributes {
                id: Uuid [pk];
                title: Option<String>;
            }
            statements {
                statement by_title {
                    up "CREATE INDEX IF NOT EXISTS indexed_notes_by_title_id ON indexed_notes (title, id)";
                    down "DROP INDEX IF EXISTS indexed_notes_by_title_id";
                }
                statement by_id {
                    up "CREATE INDEX IF NOT EXISTS indexed_notes_by_id ON indexed_notes (id)";
                    down "DROP INDEX IF EXISTS indexed_notes_by_id";
                }
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod typed_values {
    use ash_core::{Binary, CiString, Date, Decimal, Float, resource};
    use uuid::Uuid;

    resource! {
        TypedValue {
            table "typed_values";
            attributes {
                id: Uuid [pk];
                weight: Float;
                due_on: Date;
                payload: Binary;
                email: CiString;
                amount: Decimal;
            }
            identities {
                identity unique_email: [email];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod searchable_notes {
    use ash_core::{CiString, resource};
    use uuid::Uuid;

    resource! {
        SearchableNote {
            table "searchable_notes";
            attributes {
                id: Uuid [pk];
                title: String;
                email: Option<CiString>;
            }
            actions {
                read read { primary; }
                read titled_on_fire {
                    prepare filter(title.contains("on fire"));
                }
            }
        }
    }
}

pub mod device {
    use ash_core::{Inet, Vector, resource};
    use uuid::Uuid;

    resource! {
        Device {
            table "devices";
            attributes {
                id: Uuid [pk];
                address: Inet;
                embedding: Option<Vector<3>>;
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod covered_notes {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        CoveredNote {
            table "covered_notes";
            attributes {
                id: Uuid [pk];
                title: String;
                body: String;
            }
            indexes {
                index by_title: [title], include: [body];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

/// `covered_notes` with a different method and no included columns.
pub mod covered_notes_v2 {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        CoveredNote {
            table "covered_notes";
            attributes {
                id: Uuid [pk];
                title: String;
                body: String;
            }
            indexes {
                index by_title: [title], using: hash;
            }
            actions {
                read read { primary; }
            }
        }
    }
}

/// An index that names a column the table does not have.
pub mod misindexed_notes {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        MisindexedNote {
            table "misindexed_notes";
            attributes {
                id: Uuid [pk];
                title: String;
            }
            indexes {
                index by_title: [title], include: [summary];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod contact {
    use ash_core::{CiString, resource};
    use uuid::Uuid;

    resource! {
        Contact {
            table "contacts";
            attributes {
                id: Uuid [pk];
                email: CiString = "Ada@Example.com";
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod marker {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Marker {
            table "markers";
            attributes {
                id: Uuid [pk];
            }
            statements {
                statement sidecar {
                    up "CREATE TABLE marker_sidecar (id TEXT PRIMARY KEY)";
                    down "DROP TABLE IF EXISTS marker_sidecar";
                }
                // Both need the table to exist, so they run after CREATE TABLE.
                statement by_id {
                    up "CREATE INDEX IF NOT EXISTS markers_by_id ON markers (id)";
                    down "DROP INDEX IF EXISTS markers_by_id";
                }
                statement described only postgres {
                    up "COMMENT ON TABLE markers IS 'markers'";
                    down "COMMENT ON TABLE markers IS NULL";
                }
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod duplicate_statement_notes {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        DuplicateStatementNote {
            table "duplicate_statement_notes";
            attributes {
                id: Uuid [pk];
            }
            statements {
                statement prepare {
                    up "SELECT 1";
                    down "SELECT 1";
                }
                statement prepare {
                    up "SELECT 2";
                    down "SELECT 2";
                }
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod moved_files {
    pub mod folder {
        use ash_core::resource;
        use uuid::Uuid;

        resource! {
            Folder {
                table "folders";
                attributes {
                    id: Uuid [pk];
                    name: String;
                }
                actions {
                    read read { primary; }
                }
            }
        }
    }

    pub mod file {
        use super::folder::Folder;
        use ash_core::resource;
        use uuid::Uuid;

        resource! {
            File {
                table "files";
                attributes {
                    id: Uuid [pk];
                    name: String;
                    folder_id: Uuid;
                }
                relationships {
                    belongs_to folder: Folder [fk: folder_id, on_delete: restrict, on_update: cascade];
                }
                actions {
                    read read { primary; }
                }
            }
        }
    }
}

pub mod tagged_notes {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        TaggedNote {
            table "tagged_notes";
            attributes {
                id: Uuid [pk];
                body: String;
            }
            indexes {
                // hash works on text; gin needs jsonb, arrays, or tsvector.
                index by_body: [body], using: hash;
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod optional_emails {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        OptionalEmail {
            table "optional_emails";
            attributes {
                id: Uuid [pk];
                email: Option<String>;
            }
            identities {
                identity one_email: [email], nils_distinct: false;
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod live_accounts {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        LiveAccount {
            table "live_accounts";
            attributes {
                id: Uuid [pk];
                email: String;
                deleted_at: Option<String>;
            }
            identities {
                identity live_email: [email], where: "deleted_at IS NULL";
            }
            indexes {
                index active_email: [email], where: "deleted_at IS NULL";
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod duplicate_check_notes {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        DuplicateCheckNote {
            table "duplicate_check_notes";
            attributes {
                id: Uuid [pk];
                title: String;
            }
            checks {
                check titled: "title <> ''";
                check titled: "title <> 'x'";
            }
            actions {
                read read { primary; }
            }
        }
    }
}

pub mod enum_labels {
    use ash_core::{AshEnum, resource};
    use uuid::Uuid;

    #[derive(AshEnum, Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Label {
        Open,
        Closed,
    }

    resource! {
        LabeledNote {
            table "enum_labels";
            attributes {
                id: Uuid [pk];
                label: Label [enum];
            }
            actions {
                read read { primary; }
            }
        }
    }
}

/// An embedded resource stored in a JSON column.
pub mod json_profiles {
    pub mod address {
        use ash_core::resource;

        resource! {
            embedded Address {
                attributes {
                    street: String;
                    city: String;
                }
            }
        }
    }

    pub mod profile {
        use super::address::Address;
        use ash_core::resource;
        use uuid::Uuid;

        resource! {
            Profile {
                table "json_profiles";
                attributes {
                    id: Uuid [pk];
                    address: Option<Address>;
                }
                actions {
                    read read { primary; }
                }
            }
        }
    }
}

/// A user check whose name matches the generated `<attr>_one_of` enum check.
pub mod colliding_checks {
    use super::enum_labels::Label;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        CollidingNote {
            table "colliding_notes";
            attributes {
                id: Uuid [pk];
                label: Label [enum];
            }
            checks {
                check label_one_of: "label <> ''";
            }
            actions {
                read read { primary; }
            }
        }
    }
}

/// A foreign key to a column with no unique key behind it.
pub mod unkeyed_reference {
    pub mod parent {
        use ash_core::resource;
        use uuid::Uuid;

        resource! {
            Parent {
                table "unkeyed_parents";
                attributes {
                    id: Uuid [pk];
                    tenant_id: Uuid;
                }
                actions {
                    read read { primary; }
                }
            }
        }
    }

    pub mod child {
        use super::parent::Parent;
        use ash_core::resource;
        use uuid::Uuid;

        resource! {
            Child {
                table "unkeyed_children";
                attributes {
                    id: Uuid [pk];
                    tenant_id: Uuid;
                }
                relationships {
                    belongs_to parent: Parent [fk: tenant_id, references: tenant_id];
                }
                actions {
                    read read { primary; }
                }
            }
        }
    }
}

pub mod tenant_accounts {
    pub mod account {
        use ash_core::resource;
        use uuid::Uuid;

        resource! {
            Account {
                table "accounts";
                attributes {
                    id: Uuid [pk];
                    tenant_id: Uuid;
                    code: String;
                }
                identities {
                    identity tenant_code: [tenant_id, code];
                }
                actions {
                    read read { primary; }
                }
            }
        }
    }

    pub mod membership {
        use super::account::Account;
        use ash_core::resource;
        use uuid::Uuid;

        resource! {
            Membership {
                table "memberships";
                attributes {
                    id: Uuid [pk];
                    tenant_id: Uuid;
                    code: String;
                }
                relationships {
                    belongs_to account: Account [fk: [tenant_id, code], references: [tenant_id, code]];
                }
                actions {
                    read read { primary; }
                }
            }
        }
    }
}
