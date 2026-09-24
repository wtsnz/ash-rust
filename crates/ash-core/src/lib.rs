//! Kernel for a resource/action engine in the Ash style.
//!
//! Resources are static metadata. The engine runs named actions against a
//! [`DataLayer`], and read policies compile to filters so unauthorized rows
//! never come back.

mod action;
mod actor;
mod aggregate;
mod bulk;
mod changeset;
mod context;
mod data_layer;
mod engine;
mod error;
mod expr;
mod extension;
mod filter;
mod keys;
mod multi;
mod notifier;
mod pipeline;
mod policy;
mod registry;
mod rel;
mod resource;
pub mod store;
mod types;
mod value;

pub use action::{
    ActionDef, ActionKind, ActionTarget, AfterActionFn, AfterTransactionFn, ArgumentDef,
    BeforeActionFn, Change, ChangeContext, CustomChange, CustomValidation, DynamicAfterActionHook,
    DynamicAfterTransactionHook, DynamicBeforeActionHook, ManagedRelType, PersistKind,
    PreparationDef, Validation, ValidationContext,
};
pub use actor::Actor;
pub use aggregate::{AggregateDef, AggregateFilter, AggregateKind};
pub use ash_macros::{AshEnum, Resource, define, domain, resource};
pub use bulk::{BulkCreateOptions, BulkDestroyOptions, BulkResult, bulk_create, bulk_destroy};
pub use changeset::{
    AfterActionHook, AfterTransactionHook, BeforeActionHook, Changeset, IntoFieldMap,
    ManagedRelationshipSpec,
};
pub use context::Context;
pub use data_layer::{CompiledQuery, DataLayer, SchemaSupport, Sort, TransactionSupport};
pub use engine::{
    KeysetCursor, Page, Query, create, create_dynamic, destroy, destroy_dynamic, destroy_existing,
    get, handle_managed_relationships, insert, manual_create, query, run, update, update_dynamic,
    update_existing,
};
pub use error::{Error, Result};
pub use expr::{CalculationDef, Expr, apply_named, apply_named_with_args, eval};
pub use extension::ResourceExtension;
pub use filter::Filter;
pub use keys::{Aggregate, AggregateName, Attr, Calc, CalcName, FieldName, RelName, Relation};
pub use multi::{BoundMulti, IntoChangeset, Multi, MultiResult};
pub use notifier::{Notification, Notifier, SyncFnNotifier};
pub use policy::{
    Check, FieldPolicyDef, PolicyDef, PolicyEffect, PolicyWhen, authorize_field_writes,
    authorize_write, check_to_filter, compile_read_filter, redact_fields,
};
pub use registry::{BoxFuture, DataLayerRegistry, DynDataLayer, DynSchemaSupport, StoreRegistry};
pub use rel::Rel;
pub use resource::{
    AttrType, AttributeDef, CheckDef, DataLayerKind, Domain, DomainDef, IdentityDef, IndexDef,
    MultitenancyDef, MultitenancyStrategy, OnDelete, RelKind, RelationshipDef, Resource,
    ResourceDef, ResourceExt, utc_now_iso8601, utc_now_timestamp,
};
pub use store::{
    DefaultStore, HasStore, MemoryStore, PostgresStore, SqliteStore, StoreTag,
    default_store_type_id,
};
pub use types::{AshEnum, AshType, Binary, Date, Decimal, Float, UtcDateTime};
pub use value::{
    ConstValue, FieldMap, IntoOption, Value, optional_int, optional_uuid, required_string,
    required_uuid,
};

/// Typestate marker: a required action input has not been set yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputUnset;

/// Typestate marker: a required action input has been set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputSet;

#[macro_export]
macro_rules! fields {
    ($($key:expr => $value:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut map = $crate::FieldMap::new();
        $(
            map.insert(::std::string::String::from($key), $crate::Value::from($value));
        )*
        map
    }};
}
