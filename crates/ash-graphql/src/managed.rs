//! Managed relationship inputs, as AshGraphql derives them: an action argument a
//! `manage_relationship` change reads takes, in place of JSON, an input object named for
//! the resource, action and argument (`TicketOpenCommentsInput`), whose fields are what
//! the destination actions it may run take.

use ash_core::{ActionDef, ActionKind, AttrType, Change, ManagedRelType, RelKind, ResourceDef, Value as AshValue};
use async_graphql::Value as GqlValue;
use async_graphql::dynamic::{InputObject, InputValue, SchemaBuilder, TypeRef};

use crate::names::{camel, pascal};
use crate::types::{attr_type_to_type_ref, parse_input_value};

/// The input object a managed relationship argument takes.
pub(crate) struct ManagedInput {
    pub name: String,
    /// Each field, its type, and whether it's required.
    pub fields: Vec<(&'static str, AttrType, bool)>,
    /// Whether the argument takes a list of them.
    pub list: bool,
}

/// What a create or update of `resource` through `action` takes, each with whether it's
/// required: as a mutation's input holds it.
fn action_fields(action: &ActionDef, resource: &ResourceDef) -> Vec<(&'static str, AttrType, bool)> {
    let mut fields = Vec::new();
    for attr in resource.attributes.iter().filter(|attr| action.accept.contains(&attr.name)) {
        let required =
            action.kind == ActionKind::Create && !attr.allow_nil && attr.default_fn.is_none() && !attr.generated;
        fields.push((attr.name, attr.ty, required));
    }
    for arg in action.arguments {
        fields.push((arg.name, arg.ty, !arg.allow_nil && arg.default.is_none()));
    }
    fields
}

fn primary(resource: &ResourceDef, kind: ActionKind) -> Option<&'static ActionDef> {
    let of_kind = || resource.actions.iter().filter(move |a| a.kind == kind);
    of_kind().find(|a| a.primary).or_else(|| of_kind().next())
}

/// The input object `argument` of `action` takes, where a `manage_relationship` change
/// reads it and it takes a map or a list of them, as AshGraphql derives one by default.
pub(crate) fn managed_input(
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    argument: &str,
) -> Option<ManagedInput> {
    let ty = action.arguments.iter().find(|arg| arg.name == argument)?.ty;
    let list = match ty {
        AttrType::Map => false,
        AttrType::Array { of: AttrType::Map } => true,
        _ => return None,
    };
    let rel_type = action.changes.iter().find_map(|change| match change {
        Change::ManageRelationship { relationship, rel_type } if *relationship == argument => Some(*rel_type),
        _ => None,
    })?;
    let rel = resource.relationship(argument)?;
    let dest = (rel.destination)();
    // What the relationship sets itself, as AshGraphql leaves it out.
    let linked: Vec<&str> = match rel.kind {
        RelKind::ManyToMany => Vec::new(),
        _ => rel.destination_columns(),
    };
    let without_links = |fields: Vec<(&'static str, AttrType, bool)>| {
        fields.into_iter().filter(|(name, ..)| !linked.contains(name)).collect::<Vec<_>>()
    };

    // The destination actions each managed type may run, as ash-core runs them: a create
    // for each record, or with direct control, an update of each one matched by its key.
    let mut sets = Vec::new();
    if let Some(create) = primary(dest, ActionKind::Create) {
        sets.push(without_links(action_fields(create, dest)));
    }
    if rel_type == ManagedRelType::DirectControl
        && let Some(update) = primary(dest, ActionKind::Update)
    {
        sets.push(without_links(action_fields(update, dest)));
    }
    if matches!(rel_type, ManagedRelType::DirectControl | ManagedRelType::Append)
        && let Some(pk) = dest.primary_key()
    {
        sets.push(vec![(pk.name, pk.ty, true)]);
    }

    // Each field once, required only where every set requires it, as AshGraphql merges
    // the fields of the actions a managed relationship may run.
    let mut fields: Vec<(&'static str, AttrType, bool)> = Vec::new();
    for (name, ty, _) in sets.iter().flatten() {
        if fields.iter().any(|(seen, ..)| seen == name) {
            continue;
        }
        let required =
            sets.iter().all(|set| set.iter().any(|(other, _, required)| other == name && *required));
        fields.push((name, *ty, required));
    }
    fields.sort_by_key(|(name, ..)| *name);
    if fields.is_empty() {
        return None;
    }
    Some(ManagedInput {
        name: format!("{}{}{}Input", resource.name, pascal(action.name), pascal(argument)),
        fields,
        list,
    })
}

impl ManagedInput {
    /// The argument's type: the input object, or a list of them.
    pub(crate) fn type_ref(&self, allow_nil: bool) -> TypeRef {
        match (self.list, allow_nil) {
            (true, true) => TypeRef::named_nn_list(&self.name),
            (true, false) => TypeRef::named_nn_list_nn(&self.name),
            (false, true) => TypeRef::named(&self.name),
            (false, false) => TypeRef::named_nn(&self.name),
        }
    }

    /// Registers the input object.
    pub(crate) fn register(&self, builder: SchemaBuilder, resource: &ResourceDef) -> SchemaBuilder {
        let mut input = InputObject::new(&self.name);
        for (name, ty, required) in &self.fields {
            input = input.field(InputValue::new(camel(name), attr_type_to_type_ref(resource.name, name, *ty, !required)));
        }
        builder.register(input)
    }

    /// The argument's value as the action takes it: each record a map of its fields'
    /// names to their values.
    pub(crate) fn parse(&self, value: &GqlValue) -> async_graphql::Result<AshValue> {
        match value {
            GqlValue::Null => Ok(AshValue::Null),
            GqlValue::List(items) if self.list => {
                items.iter().map(|item| self.parse_record(item)).collect::<Result<Vec<_>, _>>().map(AshValue::Array)
            }
            item if self.list => Ok(AshValue::Array(vec![self.parse_record(item)?])),
            item => self.parse_record(item),
        }
    }

    fn parse_record(&self, value: &GqlValue) -> async_graphql::Result<AshValue> {
        let GqlValue::Object(given) = value else {
            return Err(async_graphql::Error::new(format!("{} takes an object", self.name)));
        };
        let mut record = ash_core::FieldMap::new();
        for (name, ty, _) in &self.fields {
            if let Some(value) = given.get(camel(name).as_str()) {
                let value = if matches!(value, GqlValue::Null) { AshValue::Null } else { parse_input_value(value, *ty)? };
                record.insert(name.to_string(), value);
            }
        }
        Ok(AshValue::Map(record))
    }
}

/// Every managed relationship input `action` takes, by argument.
pub(crate) fn managed_inputs(
    resource: &'static ResourceDef,
    action: &'static ActionDef,
) -> Vec<(&'static str, ManagedInput)> {
    action.arguments.iter().filter_map(|arg| managed_input(resource, action, arg.name).map(|m| (arg.name, m))).collect()
}
