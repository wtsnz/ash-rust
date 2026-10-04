//! Values of embedded resources, typed maps and unions, as AshGraphql types them: an
//! embedded resource is its own object type, and takes an input per attribute holding
//! it (`<Resource><Attribute>Input`); a typed map is an object type of its fields
//! (`<Name>`, `<Name>Input`); a union is a GraphQL union of an object per member
//! (`<Name><Member> { value }`), and takes an input of one field per member.

use ash_core::{AttrType, DataLayer, FieldMap, ResourceDef, Value};
use async_graphql::dynamic::{Field, FieldFuture, FieldValue, InputObject, InputValue, Object, SchemaBuilder, Union};

use crate::names::{camel, pascal};
use crate::types::{ash_value_to_graphql_value_typed, attr_type_to_type_ref, input_type_ref};

/// Whether a value of `ty` (or a list of them) is an object, not a scalar.
pub(crate) fn is_composite(ty: AttrType) -> bool {
    match ty {
        AttrType::Array { of } => is_composite(*of),
        AttrType::Embedded(_) | AttrType::TypedMap { .. } | AttrType::Union { .. } => true,
        _ => false,
    }
}

/// A union member's value, resolved through its member's object.
struct UnionValue {
    ty: AttrType,
    value: Value,
}

/// `value` of type `ty` as a field resolves it: an object for an embedded resource or
/// typed map, a union's member object, a list of them, or a scalar.
pub(crate) fn output_value(ty: AttrType, value: &Value) -> Option<FieldValue<'static>> {
    match (ty, value) {
        (_, Value::Null) => None,
        (AttrType::Array { of }, Value::Array(items)) if is_composite(*of) => {
            Some(FieldValue::list(items.iter().map(|item| output_value(*of, item).unwrap_or(FieldValue::NULL))))
        }
        (AttrType::Embedded(_) | AttrType::TypedMap { .. }, Value::Map(map)) => Some(FieldValue::owned_any(map.clone())),
        (AttrType::Union { name, members }, value) => value.union_member().and_then(|(member, held)| {
            let ty = members.iter().find(|m| m.name == member)?.ty;
            Some(FieldValue::owned_any(UnionValue { ty, value: held.clone() }).with_type(format!("{name}{}", pascal(member))))
        }),
        (ty, value) => Some(FieldValue::value(ash_value_to_graphql_value_typed(value, ty))),
    }
}

/// The object, union and input types the resources' composite values need.
#[derive(Default)]
struct Registry {
    names: Vec<String>,
    objects: Vec<Object>,
    unions: Vec<Union>,
    inputs: Vec<InputObject>,
    embedded: Vec<&'static ResourceDef>,
}

impl Registry {
    fn first(&mut self, name: &str) -> bool {
        if self.names.iter().any(|seen| seen == name) {
            return false;
        }
        self.names.push(name.to_string());
        true
    }

    /// The output types a value of `ty` needs.
    fn output(&mut self, ty: AttrType) {
        match ty {
            AttrType::Array { of } => self.output(*of),
            AttrType::Embedded(embedded) => {
                let resource = embedded.resource();
                if self.first(resource.name) {
                    self.embedded.push(resource);
                    for attr in resource.attributes {
                        self.output(attr.ty);
                    }
                }
            }
            AttrType::TypedMap { name, fields } => {
                if !self.first(name) {
                    return;
                }
                let mut object = Object::new(name);
                for field in fields {
                    let (field_name, field_ty) = (field.name, field.ty);
                    object = object.field(Field::new(
                        camel(field_name),
                        attr_type_to_type_ref(name, field_name, field_ty, field.allow_nil),
                        move |ctx| {
                            FieldFuture::new(async move {
                                Ok(ctx.parent_value.downcast_ref::<FieldMap>().and_then(|map| map.get(field_name)).and_then(|value| output_value(field_ty, value)))
                            })
                        },
                    ));
                    self.output(field_ty);
                }
                self.objects.push(object);
            }
            AttrType::Union { name, members } => {
                if !self.first(name) {
                    return;
                }
                let mut union = Union::new(name);
                // Members by name, as AshGraphql orders them.
                let mut members = members.to_vec();
                members.sort_by_key(|member| member.name);
                for member in members {
                    let object_name = format!("{name}{}", pascal(member.name));
                    let member_ty = member.ty;
                    union = union.possible_type(&object_name);
                    self.objects.push(Object::new(&object_name).field(Field::new(
                        "value",
                        attr_type_to_type_ref(name, member.name, member_ty, false),
                        move |ctx| {
                            FieldFuture::new(async move {
                                Ok(ctx.parent_value.downcast_ref::<UnionValue>().and_then(|held| output_value(held.ty, &held.value)))
                            })
                        },
                    )));
                    self.output(member_ty);
                }
                self.unions.push(union);
            }
            _ => {}
        }
    }

    /// The input types a value of `ty` given as `field` of `owner` needs.
    fn input(&mut self, owner: &str, field: &str, ty: AttrType) {
        match ty {
            AttrType::Array { of } => self.input(owner, field, *of),
            AttrType::Embedded(embedded) => {
                let name = format!("{owner}{}Input", pascal(field));
                if !self.first(&name) {
                    return;
                }
                let resource = embedded.resource();
                let mut input = InputObject::new(&name);
                for attr in resource.attributes {
                    let required = !attr.allow_nil && attr.default_fn.is_none() && !attr.generated;
                    input = input.field(InputValue::new(camel(attr.name), input_type_ref(resource.name, attr.name, attr.ty, !required)));
                    self.input(resource.name, attr.name, attr.ty);
                }
                self.inputs.push(input);
            }
            AttrType::TypedMap { name, fields } => {
                let input_name = format!("{name}Input");
                if !self.first(&input_name) {
                    return;
                }
                let mut input = InputObject::new(&input_name);
                for map_field in fields {
                    input = input.field(InputValue::new(camel(map_field.name), input_type_ref(name, map_field.name, map_field.ty, map_field.allow_nil)));
                    self.input(name, map_field.name, map_field.ty);
                }
                self.inputs.push(input);
            }
            AttrType::Union { name, members } => {
                let input_name = format!("{name}Input");
                if !self.first(&input_name) {
                    return;
                }
                let mut input = InputObject::new(&input_name);
                let mut members = members.to_vec();
                members.sort_by_key(|member| member.name);
                for member in members {
                    input = input.field(InputValue::new(camel(member.name), input_type_ref(name, member.name, member.ty, true)));
                    self.input(name, member.name, member.ty);
                }
                self.inputs.push(input);
            }
            _ => {}
        }
    }
}

/// Registers the types the composite values of `resources` need: what their attributes
/// and calculations hold, and what their attributes and actions' arguments take.
pub(crate) fn register_composites<D: DataLayer + Clone + 'static>(
    mut builder: SchemaBuilder,
    resources: &[&'static ResourceDef],
) -> SchemaBuilder {
    let mut registry = Registry::default();
    registry.names.extend(resources.iter().map(|resource| resource.name.to_string()));
    for resource in resources {
        for attr in resource.attributes {
            registry.output(attr.ty);
            registry.input(resource.name, attr.name, attr.ty);
        }
        for calc in resource.calculations {
            registry.output(calc.ty);
        }
        for action in resource.actions {
            for arg in action.arguments {
                registry.input(resource.name, arg.name, arg.ty);
            }
        }
    }
    for resource in registry.embedded {
        builder = builder.register(crate::object::build_resource_object::<D>(resource));
    }
    for object in registry.objects {
        builder = builder.register(object);
    }
    for union in registry.unions {
        builder = builder.register(union);
    }
    for input in registry.inputs {
        builder = builder.register(input);
    }
    builder
}
