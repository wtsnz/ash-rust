//! # Ash Archival
//!
//! Soft delete for `ash-rust` resources, following
//! [AshArchival](https://hexdocs.pm/ash_archival). Destroy actions archive a record by
//! setting `archived_at` instead of deleting it, and read actions hide archived records.
//!
//! ```text
//! use ash_archival::archival;
//!
//! #[archival]
//! resource! {
//!     Post {
//!         table "posts";
//!
//!         attributes {
//!             id: Uuid [pk];
//!             title: String;
//!             // `archived_at: Option<UtcDateTimeUsec>` is added for you.
//!         }
//!
//!         archive {
//!             exclude_read_actions [archived];
//!             exclude_destroy_actions [purge];
//!             archive_related [comments];
//!             unarchive_action unarchive;
//!         }
//!
//!         actions {
//!             read read { primary; }
//!             read archived { prepare filter(!archived_at.is_nil()); }
//!             destroy destroy { primary; }
//!             destroy purge {}
//!         }
//!     }
//! }
//! ```
//!
//! The transformer:
//! 1. Adds `archived_at: Option<UtcDateTimeUsec>` unless the resource declares it.
//! 2. Adds `prepare filter(archived_at.is_nil())` to every read action except
//!    `exclude_read_actions`. Relationship loads use the primary read, so they skip
//!    archived records too, as do aggregates and filters through a relationship.
//! 3. Makes every destroy action except `exclude_destroy_actions` a soft destroy that sets
//!    `archived_at`. `archive_related` relationships are destroyed first with their own
//!    primary destroy action, which archives them when they are archival as well.
//! 4. With `unarchive_action`, adds an update action that clears `archived_at`.
//! 5. Registers [`ArchiveDef`] in the resource's extensions and adds `is_archived()`.
//!
//! As in AshArchival, unique identities still cover archived rows. Add
//! `where: "archived_at IS NULL"` to an identity to let a new record reuse its key.

use std::any::Any;

use ash_core::{
    ChangeContext, Context, CustomChange, DataLayer, Error, FieldMap, Filter, Resource,
    ResourceExtension, Result, Value,
};
use uuid::Uuid;

pub use ash_archival_macros::archival;

/// Archive settings that `#[archival]` registers in `ResourceDef.extensions`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveDef {
    pub attribute: &'static str,
    pub exclude_read_actions: &'static [&'static str],
    pub exclude_destroy_actions: &'static [&'static str],
    pub archive_related: &'static [&'static str],
    pub unarchive_action: Option<&'static str>,
}

impl ArchiveDef {
    pub const fn new(
        attribute: &'static str,
        exclude_read_actions: &'static [&'static str],
        exclude_destroy_actions: &'static [&'static str],
        archive_related: &'static [&'static str],
        unarchive_action: Option<&'static str>,
    ) -> Self {
        Self {
            attribute,
            exclude_read_actions,
            exclude_destroy_actions,
            archive_related,
            unarchive_action,
        }
    }
}

impl ResourceExtension for ArchiveDef {
    fn name(&self) -> &'static str {
        "archive"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The archive settings of `R`, if it uses `#[archival]`.
pub fn archive_def<R: Resource>() -> Option<&'static ArchiveDef> {
    R::DEF.extension::<ArchiveDef>()
}

/// Sets the archive attribute to the current time.
#[derive(Debug)]
pub struct ArchiveChange {
    attribute: &'static str,
}

impl ArchiveChange {
    pub const fn new(attribute: &'static str) -> Self {
        Self { attribute }
    }
}

impl CustomChange for ArchiveChange {
    fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()> {
        ctx.fields.insert(
            self.attribute.to_string(),
            ash_core::AshType::to_value(&ash_core::UtcDateTimeUsec::now()),
        );
        Ok(())
    }
}

/// Clears the archive attribute.
#[derive(Debug)]
pub struct UnarchiveChange {
    attribute: &'static str,
}

impl UnarchiveChange {
    pub const fn new(attribute: &'static str) -> Self {
        Self { attribute }
    }
}

impl CustomChange for UnarchiveChange {
    fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()> {
        ctx.fields.insert(self.attribute.to_string(), Value::Null);
        Ok(())
    }
}

/// Restores an archived record with its `unarchive_action`.
///
/// Primary reads hide archived records, so the record is loaded with the first of
/// `exclude_read_actions`, which keeps read policies and tenancy in force.
pub async fn unarchive<R: Resource, D: DataLayer>(ctx: &Context<D>, id: Uuid) -> Result<R> {
    let def = archive_def::<R>().ok_or_else(|| {
        Error::Invalid(format!("{} does not use #[archival]", R::DEF.name))
    })?;
    let action = def.unarchive_action.ok_or_else(|| {
        Error::Invalid(format!(
            "{} needs `unarchive_action` in its archive block",
            R::DEF.name
        ))
    })?;
    let read = def.exclude_read_actions.first().ok_or_else(|| {
        Error::Invalid(format!(
            "{} needs a read action in `exclude_read_actions` to find archived records",
            R::DEF.name
        ))
    })?;
    let pk = R::DEF
        .primary_key()
        .ok_or_else(|| Error::Invalid(format!("{} has no primary key", R::DEF.name)))?;
    let record = ash_core::query::<R, D>(ctx)
        .action(read)
        .filter(Filter::eq(pk.name, id))
        .one()
        .await?;
    ash_core::update_existing::<R, D>(ctx, action, record, FieldMap::new()).await
}
