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
}

impl IdentityDef {
    pub const fn new(name: &'static str, keys: &'static [&'static str]) -> Self {
        Self {
            name,
            keys,
            message: None,
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
        }
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
    Atom { one_of: &'static [&'static str] },
    Map,
    Array,
    UtcDatetime,
    Decimal,
}

impl AttrType {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Uuid => "uuid",
            Self::String => "string",
            Self::Integer => "integer",
            Self::Boolean => "boolean",
            Self::Atom { .. } => "atom",
            Self::Map => "map",
            Self::Array => "array",
            Self::UtcDatetime => "utc_datetime",
            Self::Decimal => "decimal",
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

#[derive(Clone, Copy, Debug)]
pub struct RelationshipDef {
    pub name: &'static str,
    pub kind: RelKind,
    pub destination: fn() -> &'static ResourceDef,
    pub source_attribute: &'static str,
    pub destination_attribute: &'static str,
    pub through: Option<fn() -> &'static ResourceDef>,
    pub source_attribute_on_join_resource: Option<&'static str>,
    pub destination_attribute_on_join_resource: Option<&'static str>,
    pub on_delete: OnDelete,
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
            through: None,
            source_attribute_on_join_resource: None,
            destination_attribute_on_join_resource: None,
            on_delete: OnDelete::Nothing,
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
            through: None,
            source_attribute_on_join_resource: None,
            destination_attribute_on_join_resource: None,
            on_delete: OnDelete::Nothing,
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
            through: None,
            source_attribute_on_join_resource: None,
            destination_attribute_on_join_resource: None,
            on_delete: OnDelete::Nothing,
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
            through: Some(through),
            source_attribute_on_join_resource: Some(source_attribute_on_join_resource),
            destination_attribute_on_join_resource: Some(destination_attribute_on_join_resource),
            on_delete: OnDelete::Nothing,
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
    const ATTR_TYPE: AttrType = AttrType::Map;

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
