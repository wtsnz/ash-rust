//! Kernel for a resource/action engine in the Ash style.
//!
//! Resources are static metadata. The engine runs named actions against a
//! [`DataLayer`], and read policies compile to filters so unauthorized rows
//! never come back.

mod action;
mod actor;
mod aggregate;
mod atomic;
mod bulk;
mod changeset;
mod context;
mod data_layer;
mod engine;
mod error;
mod expr;
mod extension;
mod filter;
mod guarded;
pub mod input;
mod keys;
mod multi;
mod notifier;
mod pipeline;
mod policy;
mod registry;
mod rel;
mod resource;
#[doc(hidden)]
pub mod returned;
#[doc(hidden)]
pub mod default_value;
pub mod store;
mod types;
mod value;

pub use atomic::{Atomic, AtomicCondition, AtomicContext, AtomicExpr, AtomicUpdate};
pub use action::{
    ActionDef, ActionKind, ActionTarget, AfterActionFn, AfterTransactionFn, ArgumentDef,
    BeforeActionFn, Change, ChangeContext, CustomChange, CustomValidation, DynamicAfterActionHook,
    DynamicAfterTransactionHook, DynamicBeforeActionHook, ManagedRelType, PersistKind,
    Countable, Pagination, PreparationDef, Validation, ValidationContext, apply_argument_defaults,
};
pub use actor::Actor;
pub use aggregate::{AggregateDef, AggregateFilter, AggregateKind};
pub use ash_macros::{AshEnum, Resource, define, domain, resource};
pub use bulk::{
    BulkCreateOptions, BulkDestroyOptions, BulkResult, BulkTransaction, BulkUpdateOptions,
    bulk_create, bulk_destroy, bulk_update,
};
pub use changeset::{
    AfterActionHook, AfterTransactionHook, BeforeActionHook, Changeset, DynamicChangeset,
    DynamicChangesetHook, IntoFieldMap, ManagedRelationshipSpec,
};
pub use context::Context;
pub use data_layer::{
    CompiledQuery, DataLayer, NoDataLayer, PerKey, SchemaSupport, Sort, TransactionSupport,
};
pub use engine::{
    KeysetCursor, Page, Query, build_keyset_filter, keyset_values, create, create_dynamic, destroy, destroy_dynamic, destroy_dynamic_by_id, destroy_dynamic_via, destroy_existing,
    RelatedQuery, get, handle_managed_relationships, insert, load_related, load_related_query, manual_create, query, record_visible,
    keyset_sort, run, scope_read, update, update_dynamic, update_dynamic_expecting, update_dynamic_via, update_existing, update_existing_dynamic,
};
pub use error::{Error, Result};
pub use expr::{CalculationDef, Expr, apply_named, apply_named_with_args, eval};
pub use extension::ResourceExtension;
pub use guarded::Guarded;
pub use filter::{Filter, all_of, any_of, in_list, like_matches, text_matches};
pub use keys::{
    Aggregate, AggregateName, Attr, Calc, CalcName, FieldName, RelName, Relation, TextValue,
};
pub use multi::{BoundMulti, IntoChangeset, Multi, MultiResult};
pub use notifier::{Notification, Notifier, SyncFnNotifier};
pub use pipeline::visible_scope;
pub use policy::{
    Check, FieldPolicyDef, PolicyDef, PolicyEffect, PolicyWhen,
    authorize_write, check_to_filter, compile_read_filter, field_policy_fields, guard_input_filter,
    guard_input_sort, redact_fields,
};
pub use registry::{BoxFuture, DataLayerRegistry, DynDataLayer, DynSchemaSupport, StoreRegistry};
pub use rel::Rel;
pub use resource::{
    AttrType, AttributeDef, CheckDef, DataLayerKind, Domain, DomainDef, IdentityDef, IndexDef,
    StatementDef,
    MultitenancyDef, MultitenancyStrategy, OnDelete, OnUpdate, RelKind, RelationshipDef, Resource,
    ResourceDef, ResourceExt, utc_now_iso8601, utc_now_timestamp,
};
pub use store::{
    DefaultStore, HasStore, MemoryStore, PostgresStore, SqliteStore, StoreTag,
    default_store_type_id,
};
pub use types::{
    AshEnum, AshType, Binary, CiString, Date, Decimal, Float, Inet, TimePrecision, UtcDateTime, UtcDateTimeUsec, Vector,
    canonical_text, check_vector, compare_decimal, compare_typed, format_inet, format_vector,
    parse_vector,
};
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
