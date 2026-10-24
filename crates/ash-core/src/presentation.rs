//! Reader-specific records. Only core can construct a view from engine data.

use std::collections::{HashMap, HashSet};

use crate::{AttrType, Context, FieldMap, RelationshipDef, Resource, ResourceDef, Result, Value};

/// A hook or persistence record. It deliberately does not implement `Serialize`.
/// Trusted callbacks can inspect and change its fields before core presents it.
///
/// ```compile_fail
/// fn serialize(raw: ash_core::RawRecord) {
///     let _ = serde_json::to_value(raw);
/// }
/// ```
#[derive(Clone, Debug, Default)]
pub struct RawRecord<R = ()> {
    fields: FieldMap,
    resource: std::marker::PhantomData<fn() -> R>,
}

impl<R> From<FieldMap> for RawRecord<R> {
    fn from(fields: FieldMap) -> Self {
        Self {
            fields,
            resource: std::marker::PhantomData,
        }
    }
}

impl<R> RawRecord<R> {
    pub fn from_raw(record: RawRecord) -> Self {
        Self::from(record.fields)
    }

    pub fn into_raw(self) -> RawRecord {
        RawRecord::from(self.fields)
    }

    pub fn fields(&self) -> &FieldMap {
        &self.fields
    }

    pub fn fields_mut(&mut self) -> &mut FieldMap {
        &mut self.fields
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.fields.get(name)
    }

    pub fn insert(&mut self, name: String, value: Value) -> Option<Value> {
        self.fields.insert(name, value)
    }

    pub fn into_fields(self) -> FieldMap {
        self.fields
    }
}

/// Opaque relationship keys for core loaders. This handle cannot be serialized or
/// dereferenced to its values, including keys hidden in the source View.
///
/// ```compile_fail
/// fn serialize(view: &ash_core::View, rel: &ash_core::RelationshipDef) {
///     let _ = serde_json::to_value(view.relationship_source(rel));
/// }
/// ```
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RelationshipSource(Vec<(String, Value)>);

impl From<FieldMap> for RelationshipSource {
    fn from(fields: FieldMap) -> Self {
        let mut fields: Vec<_> = fields.into_iter().collect();
        fields.sort_by(|a, b| a.0.cmp(&b.0));
        Self(fields)
    }
}

impl RelationshipSource {
    pub(crate) fn fields(&self) -> FieldMap {
        self.0.iter().cloned().collect()
    }
}

impl std::fmt::Debug for RelationshipSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RelationshipSource").finish_non_exhaustive()
    }
}

/// Projected values and policy decisions for one reader. There is no dereference to
/// an engine record. Serialization includes projected fields only.
///
/// ```compile_fail
/// fn raw_fields(view: ash_core::View) {
///     let _: &ash_core::FieldMap = &*view;
/// }
/// ```
#[derive(Clone, Default)]
pub struct View {
    keyset: Option<Box<CursorToken>>,
    fields: FieldMap,
    forbidden: HashSet<String>,
    nested: HashMap<String, PresentedValue>,
    relationship_keys: FieldMap,
    metadata: FieldMap,
    reader: Option<std::sync::Arc<crate::Actor>>,
}

#[derive(Clone)]
struct CursorToken(String);

/// The structure of a projected composite value, including nested forbidden masks.
#[derive(Clone, Debug)]
pub enum PresentedValue {
    Scalar(Value),
    Record(View),
    Array(Vec<PresentedValue>),
    Map(HashMap<String, PresentedValue>),
}

impl PresentedValue {
    pub fn to_value(&self) -> Value {
        match self {
            Self::Scalar(value) => value.clone(),
            Self::Record(view) => Value::Map(view.fields.clone()),
            Self::Array(items) => Value::Array(items.iter().map(Self::to_value).collect()),
            Self::Map(fields) => Value::Map(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.to_value()))
                    .collect(),
            ),
        }
    }
}

impl View {
    /// An opaque server-sealed cursor, when this View was read for a keyset page.
    pub fn keyset(&self) -> Option<&str> {
        self.keyset.as_deref().map(|cursor| cursor.0.as_str())
    }

    pub(crate) fn set_keyset(&mut self, token: Option<String>) {
        self.keyset = token.map(|token| Box::new(CursorToken(token)));
    }

    pub fn fields(&self) -> &FieldMap {
        &self.fields
    }

    /// Present explicitly declared hook metadata for this View's reader. Metadata
    /// has its own type, and may itself contain protected embedded resources.
    pub fn metadata(&self, definition: &crate::MetadataDef) -> Result<Option<PresentedValue>> {
        crate::get_metadata(&self.metadata, definition.name)
            .map(|value| project_value(definition.ty, self.reader.as_deref(), value.clone()))
            .transpose()
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        self.fields.get(name)
    }

    pub fn is_forbidden(&self, name: &str) -> bool {
        self.forbidden.contains(name)
    }

    pub fn nested(&self, name: &str) -> Option<&PresentedValue> {
        self.nested.get(name)
    }

    pub fn into_fields(self) -> FieldMap {
        self.fields
    }

    pub fn into_record<R: Resource>(self) -> Result<R> {
        R::from_fields(&self.fields)
    }

    /// The source columns needed by a relationship loader. These values are never
    /// included in serialization, even when the selected key field is forbidden.
    pub fn relationship_source(&self, rel: &RelationshipDef) -> RelationshipSource {
        rel.source_columns()
            .into_iter()
            .map(|column| {
                (
                    column.to_string(),
                    self.relationship_keys
                        .get(column)
                        .cloned()
                        .unwrap_or(Value::Null),
                )
            })
            .collect::<FieldMap>()
            .into()
    }

    /// Merge fields already presented for this reader, preserving their masks.
    pub fn merge_selected(&mut self, other: &Self, names: &[String]) {
        for name in names {
            if let Some(value) = other.get(name) {
                self.fields.insert(name.clone(), value.clone());
                if other.is_forbidden(name) {
                    self.forbidden.insert(name.clone());
                } else {
                    self.forbidden.remove(name);
                }
                if let Some(value) = other.nested(name) {
                    self.nested.insert(name.clone(), value.clone());
                } else {
                    self.nested.remove(name);
                }
            }
        }
    }

    /// Keep a preloaded result alongside its parent. Only presented children can
    /// enter this cache; response adapters choose their own cache keys.
    pub fn with_loaded(&mut self, key: String, rows: Vec<View>, to_one: bool) {
        let value = if to_one {
            rows.into_iter()
                .next()
                .map(PresentedValue::Record)
                .unwrap_or(PresentedValue::Scalar(Value::Null))
        } else {
            PresentedValue::Array(rows.into_iter().map(PresentedValue::Record).collect())
        };
        self.nested.insert(key, value);
    }
}

impl std::fmt::Debug for View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("View")
            .field("fields", &self.fields)
            .field("forbidden", &self.forbidden)
            .finish_non_exhaustive()
    }
}

impl serde::Serialize for View {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        self.fields.serialize(serializer)
    }
}

impl<D> Context<D> {
    /// Finish an engine record for this context's reader. Selection happens after
    /// all policies have inspected the intact record.
    pub fn present(
        &self,
        resource: &ResourceDef,
        record: RawRecord,
        select: Option<&[String]>,
    ) -> Result<View> {
        present(resource, self.authorization_actor(), record, select)
    }

    /// Present an action's typed result, including resources nested in composites.
    pub fn present_value(&self, ty: AttrType, value: Value) -> Result<PresentedValue> {
        project_value(ty, self.authorization_actor(), value)
    }
}

fn present(
    resource: &ResourceDef,
    actor: Option<&crate::Actor>,
    record: RawRecord,
    select: Option<&[String]>,
) -> Result<View> {
    let forbidden: HashSet<String> = crate::hidden_fields(resource, actor, record.fields())?
        .into_iter()
        .map(str::to_string)
        .collect();
    let relationship_keys = resource
        .relationships
        .iter()
        .flat_map(RelationshipDef::source_columns)
        .filter_map(|key| {
            record
                .fields
                .get(key)
                .map(|value| (key.to_string(), value.clone()))
        })
        .collect();
    let metadata = record
        .fields
        .iter()
        .filter(|(key, _)| key.starts_with("__metadata__:"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut view = View {
        forbidden,
        relationship_keys,
        metadata,
        reader: actor.cloned().map(std::sync::Arc::new),
        ..View::default()
    };
    for (name, value) in record.fields {
        if select.is_some_and(|fields| !fields.contains(&name)) {
            continue;
        }
        let ty = resource
            .attribute(&name)
            .map(|field| field.ty)
            .or_else(|| resource.calculation(&name).map(|field| field.ty))
            .or_else(|| resource.aggregate(&name).map(|field| field.ty));
        let projected = if view.is_forbidden(&name) {
            PresentedValue::Scalar(Value::Null)
        } else if let Some(ty) = ty {
            project_value(ty, actor, value)?
        } else if let Some(rel) = resource.relationship(&name) {
            project_relationship((rel.destination)(), actor, value)?
        } else {
            // Hook metadata is available to explicit metadata renderers only.
            continue;
        };
        view.fields.insert(name.clone(), projected.to_value());
        view.nested.insert(name, projected);
    }
    Ok(view)
}

fn project_relationship(
    resource: &ResourceDef,
    actor: Option<&crate::Actor>,
    value: Value,
) -> Result<PresentedValue> {
    match value {
        Value::Map(fields) => Ok(PresentedValue::Record(present(
            resource,
            actor,
            fields.into(),
            None,
        )?)),
        Value::Array(values) => Ok(PresentedValue::Array(
            values
                .into_iter()
                .map(|value| project_relationship(resource, actor, value))
                .collect::<Result<_>>()?,
        )),
        value => Ok(PresentedValue::Scalar(value)),
    }
}

fn project_value(
    ty: AttrType,
    actor: Option<&crate::Actor>,
    value: Value,
) -> Result<PresentedValue> {
    match (ty, value) {
        (AttrType::Embedded(embedded), Value::Map(fields)) => Ok(PresentedValue::Record(present(
            embedded.resource(),
            actor,
            fields.into(),
            None,
        )?)),
        (AttrType::Array { of }, Value::Array(items)) => Ok(PresentedValue::Array(
            items
                .into_iter()
                .map(|item| project_value(*of, actor, item))
                .collect::<Result<_>>()?,
        )),
        (AttrType::TypedMap { fields, .. }, Value::Map(mut values)) => {
            let mut projected = HashMap::new();
            for field in fields {
                if let Some(value) = values.remove(field.name) {
                    projected.insert(
                        field.name.to_string(),
                        project_value(field.ty, actor, value)?,
                    );
                }
            }
            Ok(PresentedValue::Map(projected))
        }
        (AttrType::Union { members, .. }, value) if !value.is_null() => {
            let Some((member, held)) = value.union_member() else {
                return Err(crate::Error::Invalid("invalid union result".into()));
            };
            let ty = members
                .iter()
                .find(|candidate| candidate.name == member)
                .ok_or_else(|| crate::Error::Invalid("unknown union result member".into()))?
                .ty;
            let mut fields = HashMap::new();
            fields.insert(
                "type".into(),
                PresentedValue::Scalar(Value::String(member.to_string())),
            );
            fields.insert("value".into(), project_value(ty, actor, held.clone())?);
            Ok(PresentedValue::Map(fields))
        }
        (_, value) => Ok(PresentedValue::Scalar(value)),
    }
}
