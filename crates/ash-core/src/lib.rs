//! Kernel for a resource/action engine in the Ash style.
//!
//! Resources are static metadata. The engine runs named actions against a
//! [`DataLayer`], and read policies compile to filters so unauthorized rows
//! never come back.

mod action;
mod actor;
mod aggregate;
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
mod types;
mod value;

pub use action::{
    ActionDef, ActionKind, ArgumentDef, Change, ChangeContext, CustomChange, CustomValidation,
    PersistKind, PreparationDef, Validation, ValidationContext,
};
pub use actor::Actor;
pub use aggregate::{AggregateDef, AggregateFilter, AggregateKind};
pub use ash_macros::{AshEnum, Resource, define, domain, resource};
pub use changeset::{AfterActionHook, AfterTransactionHook, BeforeActionHook, Changeset};
pub use context::Context;
pub use data_layer::{CompiledQuery, DataLayer, SchemaSupport, Sort, TransactionSupport};
pub use engine::{
    KeysetCursor, Page, Query, create, destroy, destroy_existing, get, insert, manual_create, query, run,
    update, update_existing,
};
pub use error::{Error, Result};
pub use expr::{CalculationDef, Expr, apply_named, eval};
pub use extension::ResourceExtension;
pub use filter::Filter;
pub use keys::{Aggregate, AggregateName, Attr, Calc, CalcName, FieldName, RelName, Relation};
pub use multi::{BoundMulti, IntoChangeset, Multi, MultiResult};
pub use notifier::{Notification, Notifier, SyncFnNotifier};
pub use policy::{
    Check, FieldPolicyDef, PolicyDef, PolicyEffect, PolicyWhen, authorize_write, check_to_filter,
    compile_read_filter,
};
pub use registry::{BoxFuture, DataLayerRegistry, DynDataLayer, DynSchemaSupport};
pub use rel::Rel;
pub use resource::{
    AttrType, AttributeDef, DataLayerKind, Domain, DomainDef, IdentityDef, RelKind,
    RelationshipDef, Resource, ResourceDef, ResourceExt, utc_now_iso8601, utc_now_timestamp,
};
pub use types::{AshEnum, AshType};
pub use value::{
    ConstValue, FieldMap, IntoOption, Value, optional_int, optional_uuid, required_string,
    required_uuid,
};

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
