use std::future::Future;
use std::pin::Pin;

use uuid::Uuid;

use crate::action::{ActionDef, ActionKind};
use crate::aggregate::AggregateDef;
use crate::context::Context;
use crate::data_layer::DataLayer;
use crate::error::{Error, Result};
use crate::expr::CalculationDef;
use crate::extension::ResourceExtension;
use crate::notifier::Notifier;
use crate::policy::{FieldPolicyDef, PolicyDef};
use crate::value::FieldMap;

#[derive(Clone, Copy, Debug)]
pub struct DomainDef {
    pub name: &'static str,
    pub resources: &'static [&'static ResourceDef],
}

impl DomainDef {
    pub fn resource(&self, name: &str) -> Option<&'static ResourceDef> {
        self.resources.iter().copied().find(|r| r.name == name)
    }

    pub fn has_resource(&self, name: &str) -> bool {
        self.resource(name).is_some()
    }

    pub fn validate(&self) -> Result<()> {
        for resource in self.resources {
            for rel in resource.relationships {
                let dest = (rel.destination)();
                if dest.name.is_empty() {
                    return Err(Error::Invalid(format!(
                        "relationship `{}` on `{}` has empty destination name",
                        rel.name, resource.name
                    )));
                }
            }
        }
        Ok(())
    }
}

pub trait Domain: Sized + 'static {
    const DEF: DomainDef;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataLayerKind {
    Memory,
    Sqlite,
    Postgres,
    Embedded,
    Custom(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityDef {
    pub name: &'static str,
    pub keys: &'static [&'static str],
    pub message: Option<&'static str>,
    /// SQL predicate for a partial unique index (`CREATE UNIQUE INDEX ... WHERE ...`).
    pub predicate: Option<&'static str>,
    /// When false, Postgres emits `UNIQUE NULLS NOT DISTINCT`. SQLite has no equivalent and keeps the default unique index.
    pub nils_distinct: bool,
}

impl IdentityDef {
    pub const fn with_nils_distinct(mut self, nils_distinct: bool) -> Self {
        self.nils_distinct = nils_distinct;
        self
    }

    pub const fn new(name: &'static str, keys: &'static [&'static str]) -> Self {
        Self {
            name,
            keys,
            message: None,
            predicate: None,
            nils_distinct: true,
        }
    }

    pub const fn with_message(
        name: &'static str,
        keys: &'static [&'static str],
        message: &'static str,
    ) -> Self {
        Self {
            name,
            keys,
            message: Some(message),
            predicate: None,
            nils_distinct: true,
        }
    }

    pub const fn with_predicate(mut self, predicate: &'static str) -> Self {
        self.predicate = Some(predicate);
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexDef {
    pub name: &'static str,
    pub keys: &'static [&'static str],
    /// SQL predicate for a partial index (`CREATE INDEX ... WHERE ...`).
    pub predicate: Option<&'static str>,
    /// Index access method. `None` and `btree` stay the default and are omitted from SQL. Other methods are emitted as `USING` on Postgres only.
    pub method: Option<&'static str>,
    /// Extra columns stored in the index for index-only scans (`INCLUDE (...)`), Postgres only.
    pub include: &'static [&'static str],
}

impl IndexDef {
    pub const fn new(name: &'static str, keys: &'static [&'static str]) -> Self {
        Self {
            name,
            keys,
            predicate: None,
            method: None,
            include: &[],
        }
    }

    pub const fn with_predicate(mut self, predicate: &'static str) -> Self {
        self.predicate = Some(predicate);
        self
    }

    pub const fn with_method(mut self, method: &'static str) -> Self {
        self.method = Some(method);
        self
    }

    pub const fn with_include(mut self, include: &'static [&'static str]) -> Self {
        self.include = include;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CheckDef {
    pub name: &'static str,
    pub expression: &'static str,
}

/// Raw SQL run beside the generated table migration.
///
/// An empty `dialects` list means every dialect. Otherwise the statement is
/// emitted only when `SqlDialect::name` is in the list, so Postgres-only SQL
/// such as `CREATE EXTENSION` is not sent to SQLite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatementDef {
    pub name: &'static str,
    pub dialects: &'static [&'static str],
    pub up: &'static str,
    pub down: &'static str,
}

impl CheckDef {
    pub const fn new(name: &'static str, expression: &'static str) -> Self {
        Self { name, expression }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultitenancyStrategy {
    Attribute(&'static str),
    Context,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MultitenancyDef {
    pub strategy: MultitenancyStrategy,
    pub global: bool,
}

impl MultitenancyDef {
    pub const fn attribute(attribute: &'static str) -> Self {
        Self {
            strategy: MultitenancyStrategy::Attribute(attribute),
            global: false,
        }
    }

    pub const fn context() -> Self {
        Self {
            strategy: MultitenancyStrategy::Context,
            global: false,
        }
    }

    pub const fn with_global(mut self, global: bool) -> Self {
        self.global = global;
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ResourceDef {
    pub name: &'static str,
    pub table: &'static str,
    pub attributes: &'static [AttributeDef],
    pub relationships: &'static [RelationshipDef],
    pub actions: &'static [ActionDef],
    pub policies: &'static [PolicyDef],
    pub field_policies: &'static [FieldPolicyDef],
    pub calculations: &'static [CalculationDef],
    pub aggregates: &'static [AggregateDef],
    pub extensions: &'static [&'static dyn ResourceExtension],
    pub notifiers: &'static [&'static dyn Notifier],
    pub identities: &'static [IdentityDef],
    pub indexes: &'static [IndexDef],
    pub checks: &'static [CheckDef],
    pub statements: &'static [StatementDef],
    pub embedded: bool,
    pub data_layer: DataLayerKind,
    pub timestamps: Option<(&'static str, &'static str)>,
    pub store_type_id: fn() -> std::any::TypeId,
    pub store_name: &'static str,
    pub multitenancy: Option<MultitenancyDef>,
}

impl ResourceDef {
    pub fn table_name(&self) -> &'static str {
        if self.table.is_empty() {
            self.name
        } else {
            self.table
        }
    }

    pub fn identity(&self, name: &str) -> Option<&IdentityDef> {
        self.identities
            .iter()
            .find(|identity| identity.name == name)
    }

    pub fn is_embedded(&self) -> bool {
        self.embedded || self.data_layer == DataLayerKind::Embedded
    }

    pub fn timestamps(&self) -> Option<(&'static str, &'static str)> {
        self.timestamps
    }

    pub fn has_timestamps(&self) -> bool {
        self.timestamps.is_some()
    }

    pub fn store_type_id(&self) -> std::any::TypeId {
        (self.store_type_id)()
    }

    pub fn store_name(&self) -> &'static str {
        self.store_name
    }

    pub fn attribute(&self, name: &str) -> Option<&AttributeDef> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
    }

    pub fn action(&self, name: &str) -> Option<&ActionDef> {
        self.actions.iter().find(|action| action.name == name)
    }

    pub fn calculation(&self, name: &str) -> Option<&CalculationDef> {
        self.calculations
            .iter()
            .find(|calculation| calculation.name == name)
    }

    pub fn aggregate(&self, name: &str) -> Option<&AggregateDef> {
        self.aggregates
            .iter()
            .find(|aggregate| aggregate.name == name)
    }

    pub fn extension<T: 'static>(&self) -> Option<&'static T> {
        self.extensions
            .iter()
            .find_map(|ext| ext.as_any().downcast_ref::<T>())
    }

    pub fn optimistic_lock_attribute(&self) -> Option<&'static str> {
        self.attributes
            .iter()
            .find(|attribute| attribute.version)
            .map(|attribute| attribute.name)
    }

    pub fn relationship(&self, name: &str) -> Option<&RelationshipDef> {
        self.relationships
            .iter()
            .find(|relationship| relationship.name == name)
    }

    pub fn primary_key(&self) -> Option<&AttributeDef> {
        self.attributes
            .iter()
            .find(|attribute| attribute.primary_key)
    }

    /// Filters from the primary read's `prepare filter(...)` steps. Following Ash, every
    /// read of the resource sees them: relationship loads, aggregates, and filters that
    /// reach it through a relationship.
    pub fn primary_read_filter(&self) -> Option<crate::filter::Filter> {
        self.read_filter(self.primary_read()?)
    }

    /// The filters `read`'s preparations add: the records it reaches.
    pub fn read_filter(&self, read: &crate::action::ActionDef) -> Option<crate::filter::Filter> {
        let filters: Vec<crate::filter::Filter> = read
            .preparations
            .iter()
            .filter_map(|prep| match prep {
                crate::action::PreparationDef::Filter(build) => Some(build()),
                _ => None,
            })
            .collect();
        if filters.is_empty() {
            None
        } else {
            Some(crate::filter::Filter::and(filters))
        }
    }

    /// Limits a read in `tenant` to that tenant's rows of an attribute-tenant resource,
    /// whether it reads the resource directly or through a relationship. Context-tenant
    /// resources are kept apart by the data layer instead.
    pub fn tenant_filter(&self, tenant: Option<&str>) -> Option<crate::filter::Filter> {
        match (self.multitenancy?.strategy, tenant) {
            (MultitenancyStrategy::Attribute(attribute), Some(tenant)) => Some(
                crate::filter::Filter::eq(attribute, crate::value::Value::String(tenant.to_string())),
            ),
            _ => None,
        }
    }

    /// The read this resource is read through when no action is named: typed queries,
    /// relationship loads and GraphQL all use it. That's its primary read, or for a
    /// resource that declares no read action, an implicit `read` without preparations,
    /// through which its read policies still apply.
    pub fn default_read(&self) -> &ActionDef {
        // As Ash's default read (`defaults [:read]`): paging by keyset or offset, when asked.
        static IMPLICIT_READ: ActionDef = ActionDef::read("read").pagination(
            crate::action::Pagination::keyset()
                .and_offset()
                .countable(crate::action::Countable::Yes)
                .required(false),
        );
        self.primary_read().unwrap_or(&IMPLICIT_READ)
    }

    pub fn primary_read(&self) -> Option<&ActionDef> {
        self.actions
            .iter()
            .find(|action| action.kind == ActionKind::Read && action.primary)
            .or_else(|| {
                self.actions
                    .iter()
                    .find(|action| action.kind == ActionKind::Read)
            })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AttributeDef {
    pub name: &'static str,
    pub ty: AttrType,
    pub primary_key: bool,
    pub allow_nil: bool,
    pub generated: bool,
    pub version: bool,
    pub default_fn: Option<fn() -> crate::value::Value>,
}

impl AttributeDef {
    pub const fn uuid_pk(name: &'static str) -> Self {
        Self {
            name,
            ty: AttrType::Uuid,
            primary_key: true,
            allow_nil: false,
            generated: true,
            version: false,
            default_fn: None,
        }
    }

    pub const fn required(name: &'static str, ty: AttrType) -> Self {
        Self {
            name,
            ty,
            primary_key: false,
            allow_nil: false,
            generated: false,
            version: false,
            default_fn: None,
        }
    }

    pub const fn optional(name: &'static str, ty: AttrType) -> Self {
        Self {
            name,
            ty,
            primary_key: false,
            allow_nil: true,
            generated: false,
            version: false,
            default_fn: None,
        }
    }

    pub const fn version(name: &'static str) -> Self {
        Self {
            name,
            ty: AttrType::Integer,
            primary_key: false,
            allow_nil: false,
            generated: false,
            version: true,
            default_fn: None,
        }
    }

    pub const fn generated(name: &'static str, ty: AttrType) -> Self {
        Self {
            name,
            ty,
            primary_key: false,
            allow_nil: false,
            generated: true,
            version: false,
            default_fn: None,
        }
    }

    pub const fn with_default(
        name: &'static str,
        ty: AttrType,
        default_fn: fn() -> crate::value::Value,
    ) -> Self {
        Self {
            name,
            ty,
            primary_key: false,
            allow_nil: false,
            generated: false,
            version: false,
            default_fn: Some(default_fn),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrType {
    Uuid,
    String,
    Integer,
    Boolean,
    /// One of a fixed set of values. A named atom is an enum type of its own (an
    /// `AshEnum`, as an `Ash.Type.Enum` in Elixir): GraphQL gives it an enum type of that
    /// name. An unnamed one is just constrained text there, as Ash's `:atom` is.
    Atom {
        one_of: &'static [&'static str],
        name: Option<&'static str>,
    },
    Map,
    /// A list of values of one type, as Ash's `{:array, type}`.
    Array {
        of: &'static AttrType,
    },
    UtcDatetime { precision: crate::types::TimePrecision },
    Decimal,
    Float,
    Date,
    Binary,
    CiString,
    Inet,
    Vector { dimensions: u32 },
    /// An embedded resource, as an attribute holding one has it: its fields, as a map.
    Embedded(EmbeddedType),
    /// A map of declared fields, as Ash's `:map` with `fields` constraints (or a typed
    /// struct): `#[derive(AshTypedMap)]` on a struct.
    TypedMap { name: &'static str, fields: &'static [MapField] },
    /// One of several typed members, as Ash's `Ash.Type.Union`, held as `{type, value}`:
    /// `#[derive(AshUnion)]` on an enum.
    Union { name: &'static str, members: &'static [UnionMember] },
}

/// The embedded resource an [`AttrType::Embedded`] attribute holds.
#[derive(Clone, Copy)]
pub struct EmbeddedType(pub &'static ResourceDef);

impl EmbeddedType {
    pub const fn resource(self) -> &'static ResourceDef {
        self.0
    }
}

impl PartialEq for EmbeddedType {
    fn eq(&self, other: &Self) -> bool {
        self.0.name == other.0.name
    }
}

impl Eq for EmbeddedType {}

impl std::fmt::Debug for EmbeddedType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Embedded").field(&self.0.name).finish()
    }
}

/// A field of an [`AttrType::TypedMap`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MapField {
    pub name: &'static str,
    pub ty: AttrType,
    pub allow_nil: bool,
}

/// A member of an [`AttrType::Union`]: its name, and the type its value is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnionMember {
    pub name: &'static str,
    pub ty: AttrType,
}

impl AttrType {
    /// The fields a value of this type holds, where it holds declared ones: an embedded
    /// resource's attributes, or a typed map's fields.
    pub fn fields(self) -> Option<Vec<MapField>> {
        match self {
            Self::Embedded(embedded) => Some(
                embedded
                    .resource()
                    .attributes
                    .iter()
                    .map(|attr| MapField { name: attr.name, ty: attr.ty, allow_nil: attr.allow_nil })
                    .collect(),
            ),
            Self::TypedMap { fields, .. } => Some(fields.to_vec()),
            _ => None,
        }
    }

    /// Whether a value of this type is held as a map: a map, an embedded resource, a
    /// typed map or a union.
    pub const fn is_map_like(self) -> bool {
        matches!(self, Self::Map | Self::Embedded(_) | Self::TypedMap { .. } | Self::Union { .. })
    }

    /// A UTC datetime to the second, as Ash's `:utc_datetime`.
    pub const UTC_DATETIME: Self = Self::UtcDatetime {
        precision: crate::types::TimePrecision::Second,
    };
    /// A UTC datetime to the microsecond, as Ash's `:utc_datetime_usec`.
    pub const UTC_DATETIME_USEC: Self = Self::UtcDatetime {
        precision: crate::types::TimePrecision::Microsecond,
    };

    pub const fn name(self) -> &'static str {
        match self {
            Self::Uuid => "uuid",
            Self::String => "string",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Atom { .. } => "atom",
            Self::Map => "map",
            Self::Array { .. } => "array",
            Self::UtcDatetime {
                precision: crate::types::TimePrecision::Second,
            } => "utc_datetime",
            Self::UtcDatetime {
                precision: crate::types::TimePrecision::Microsecond,
            } => "utc_datetime_usec",
            Self::Decimal => "decimal",
            Self::Float => "float",
            Self::Date => "date",
            Self::Binary => "binary",
            Self::CiString => "ci_string",
            Self::Inet => "inet",
            Self::Vector { .. } => "vector",
            Self::Embedded(_) | Self::TypedMap { .. } => "map",
            Self::Union { .. } => "union",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OnDelete {
    #[default]
    Nothing,
    Cascade,
    Nilify,
    Restrict,
}

/// Referential action for `ON UPDATE`. `Nothing` is `NO ACTION` and is omitted from generated SQL.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OnUpdate {
    #[default]
    Nothing,
    Cascade,
    Nilify,
    Restrict,
}

#[derive(Clone, Copy, Debug)]
pub struct RelationshipDef {
    pub name: &'static str,
    pub kind: RelKind,
    pub destination: fn() -> &'static ResourceDef,
    pub source_attribute: &'static str,
    pub destination_attribute: &'static str,
    /// Extra key columns on this resource. Empty means [`Self::source_attribute`] alone.
    pub source_attributes: &'static [&'static str],
    /// Matching columns on the destination. Empty means [`Self::destination_attribute`] alone.
    pub destination_attributes: &'static [&'static str],
    pub through: Option<fn() -> &'static ResourceDef>,
    pub source_attribute_on_join_resource: Option<&'static str>,
    pub destination_attribute_on_join_resource: Option<&'static str>,
    pub on_delete: OnDelete,
    pub on_update: OnUpdate,
}

fn key_values(fields: &FieldMap, columns: &[&str]) -> Option<Vec<crate::value::Value>> {
    columns
        .iter()
        .map(|column| match fields.get(*column) {
            Some(value) if !value.is_null() => Some(value.clone()),
            _ => None,
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelKind {
    BelongsTo,
    HasMany,
    HasOne,
    ManyToMany,
}

impl RelationshipDef {
    pub const fn with_on_delete(mut self, on_delete: OnDelete) -> Self {
        self.on_delete = on_delete;
        self
    }

    pub const fn with_on_update(mut self, on_update: OnUpdate) -> Self {
        self.on_update = on_update;
        self
    }

    /// Sets a composite key. The first column of each side is also stored in the single-column fields.
    pub const fn with_keys(
        mut self,
        source: &'static [&'static str],
        destination: &'static [&'static str],
    ) -> Self {
        if let Some(first) = source.first() {
            self.source_attribute = first;
        }
        if let Some(first) = destination.first() {
            self.destination_attribute = first;
        }
        self.source_attributes = source;
        self.destination_attributes = destination;
        self
    }

    pub fn source_columns(&self) -> Vec<&'static str> {
        if self.source_attributes.is_empty() {
            vec![self.source_attribute]
        } else {
            self.source_attributes.to_vec()
        }
    }

    pub fn destination_columns(&self) -> Vec<&'static str> {
        if self.destination_attributes.is_empty() {
            vec![self.destination_attribute]
        } else {
            self.destination_attributes.to_vec()
        }
    }

    /// `(column on this resource, column on the destination)` for each key column.
    pub fn key_pairs(&self) -> Vec<(&'static str, &'static str)> {
        self.source_columns()
            .into_iter()
            .zip(self.destination_columns())
            .collect()
    }

    /// This side's key values in `source`, or `None` when any of them is null or missing.
    pub fn source_key(&self, source: &FieldMap) -> Option<Vec<crate::value::Value>> {
        key_values(source, &self.source_columns())
    }

    /// The destination's key values in `destination`, or `None` when any is null or missing.
    pub fn destination_key(&self, destination: &FieldMap) -> Option<Vec<crate::value::Value>> {
        key_values(destination, &self.destination_columns())
    }

    /// Selects the destination rows linked to `source`, or `None` when its key is null,
    /// since a null key links to nothing.
    pub fn destination_filter(&self, source: &FieldMap) -> Option<crate::filter::Filter> {
        let key = self.source_key(source)?;
        Some(crate::filter::Filter::and(
            self.destination_columns()
                .into_iter()
                .zip(key)
                .map(|(column, value)| crate::filter::Filter::eq(column, value)),
        ))
    }

    pub const fn belongs_to(
        name: &'static str,
        destination: fn() -> &'static ResourceDef,
        source_attribute: &'static str,
    ) -> Self {
        Self {
            name,
            kind: RelKind::BelongsTo,
            destination,
            source_attribute,
            destination_attribute: "id",
            source_attributes: &[],
            destination_attributes: &[],
            through: None,
            source_attribute_on_join_resource: None,
            destination_attribute_on_join_resource: None,
            on_delete: OnDelete::Nothing,
            on_update: OnUpdate::Nothing,
        }
    }

    pub const fn has_many(
        name: &'static str,
        destination: fn() -> &'static ResourceDef,
        destination_attribute: &'static str,
    ) -> Self {
        Self {
            name,
            kind: RelKind::HasMany,
            destination,
            source_attribute: "id",
            destination_attribute,
            source_attributes: &[],
            destination_attributes: &[],
            through: None,
            source_attribute_on_join_resource: None,
            destination_attribute_on_join_resource: None,
            on_delete: OnDelete::Nothing,
            on_update: OnUpdate::Nothing,
        }
    }

    pub const fn has_one(
        name: &'static str,
        destination: fn() -> &'static ResourceDef,
        destination_attribute: &'static str,
    ) -> Self {
        Self {
            name,
            kind: RelKind::HasOne,
            destination,
            source_attribute: "id",
            destination_attribute,
            source_attributes: &[],
            destination_attributes: &[],
            through: None,
            source_attribute_on_join_resource: None,
            destination_attribute_on_join_resource: None,
            on_delete: OnDelete::Nothing,
            on_update: OnUpdate::Nothing,
        }
    }

    pub const fn many_to_many(
        name: &'static str,
        destination: fn() -> &'static ResourceDef,
        through: fn() -> &'static ResourceDef,
        source_attribute_on_join_resource: &'static str,
        destination_attribute_on_join_resource: &'static str,
    ) -> Self {
        Self {
            name,
            kind: RelKind::ManyToMany,
            destination,
            source_attribute: "id",
            destination_attribute: "id",
            source_attributes: &[],
            destination_attributes: &[],
            through: Some(through),
            source_attribute_on_join_resource: Some(source_attribute_on_join_resource),
            destination_attribute_on_join_resource: Some(destination_attribute_on_join_resource),
            on_delete: OnDelete::Nothing,
            on_update: OnUpdate::Nothing,
        }
    }
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not an Ash resource",
    label = "not an Ash resource",
    note = "define it with `resource! {{ {Self} {{ ... }} }}` or check imports"
)]
pub trait Resource: Sized + Clone + Send + Sync + 'static {
    type Store: crate::store::StoreTag;
    const DEF: ResourceDef;

    fn id(&self) -> Uuid;
    fn to_fields(&self) -> FieldMap;
    fn from_fields(fields: &FieldMap) -> Result<Self>;

    fn attach(&mut self, name: &str, _related: Vec<FieldMap>) -> Result<()> {
        Err(Error::Invalid(format!(
            "unknown relationship `{name}` on {}",
            Self::DEF.name
        )))
    }

    /// Runs the generic action `action` with `input`, its arguments by name, as its own
    /// `run` does: authorized by its policies, in a transaction if it takes one, its result
    /// as a value. An API serving the action by name runs it so. An action with no `run`
    /// of its own: [`Error::ManualRequired`].
    fn run_generic<'a, D: crate::data_layer::TransactionSupport + 'static>(
        _ctx: &'a Context<D>,
        action: &'a str,
        _input: FieldMap,
    ) -> Pin<Box<dyn Future<Output = Result<crate::value::Value>> + Send + 'a>> {
        Box::pin(async move {
            let action = Self::DEF.action(action).map(|a| a.name).unwrap_or("unknown");
            Err(Error::ManualRequired { action })
        })
    }
}

/// Ergonomic record lifecycle extensions for instances of [`Resource`].
pub trait ResourceExt: Resource {
    /// Reload this record from the data layer to get its latest persisted state.
    fn reload<'a, D: DataLayer>(
        &'a self,
        ctx: &'a Context<D>,
    ) -> Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>> {
        Box::pin(async move { crate::engine::get::<Self, D>(ctx, self.id()).await })
    }

    /// Destroy this record using its primary destroy action (or the first destroy action found).
    fn destroy<'a, D: DataLayer>(
        &'a self,
        ctx: &'a Context<D>,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let primary_destroy = Self::DEF
                .actions
                .iter()
                .find(|a| a.kind == ActionKind::Destroy && a.primary)
                .or_else(|| {
                    Self::DEF
                        .actions
                        .iter()
                        .find(|a| a.kind == ActionKind::Destroy)
                })
                .ok_or_else(|| {
                    Error::Invalid(format!(
                        "no destroy action found on resource `{}`",
                        Self::DEF.name
                    ))
                })?;

            crate::engine::destroy_existing::<Self, D>(ctx, primary_destroy.name, self.clone())
                .await
        })
    }
}

impl<R: Resource> ResourceExt for R {}

impl<T: Resource> crate::types::AshType for T {
    const ATTR_TYPE: AttrType = AttrType::Embedded(EmbeddedType(&T::DEF));

    fn to_value(&self) -> crate::value::Value {
        crate::value::Value::Map(self.to_fields())
    }

    fn from_value(value: &crate::value::Value) -> Result<Self> {
        match value {
            crate::value::Value::Map(m) => Self::from_fields(m),
            _ => Err(Error::Invalid("expected map".into())),
        }
    }
}

/// Formats the current UTC system time as an ISO 8601 string (e.g. "2026-09-02T16:55:00Z").
pub fn utc_now_iso8601() -> String {
    let now = std::time::SystemTime::now();
    let total_secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let sec = (total_secs % 60) as u32;
    let total_mins = total_secs / 60;
    let min = (total_mins % 60) as u32;
    let total_hours = total_mins / 60;
    let hour = (total_hours % 24) as u32;
    let total_days = (total_hours / 24) as i64;

    // Howard Hinnant's algorithm for converting Unix epoch days to civil calendar date
    let z = total_days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Returns the current UTC system time as seconds since Unix epoch.
pub fn utc_now_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
