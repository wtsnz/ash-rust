use ash_core::create_dynamic;
use ash_core::destroy_dynamic;
use ash_core::redact_fields;
use ash_core::update_dynamic;
use ash_core::{ActionDef, ActionKind, AttrType, CompiledQuery, DataLayer, Error as AshError, FieldMap, Filter, ResourceDef, Value};
use async_graphql::dynamic::*;
use uuid::Uuid;

use crate::error::{MUTATION_ERROR, UserError};
use crate::names::{camel, pascal};
use crate::types::{attr_type_to_type_ref, parse_input_val};

#[derive(Clone)]
pub struct MutationPayload {
    pub result: Option<FieldMap>,
    pub errors: Vec<UserError>,
}

pub fn capitalize_first(s: &str) -> String {
    crate::names::pascal(s)
}

pub fn to_camel_case(s: &str) -> String {
    crate::names::camel(s)
}

/// `recall` on `Cab` → `recallCab`.
pub fn mutation_name(action_name: &str, resource_name: &str) -> String {
    format!("{}{}", camel(action_name), resource_name)
}

/// `recall` on `Cab` → `RecallCabInput`.
pub fn mutation_input_name(action_name: &str, resource_name: &str) -> String {
    format!("{}{}Input", pascal(action_name), resource_name)
}

/// `recall` on `Cab` → `RecallCabResult`.
pub fn mutation_payload_name(action_name: &str, resource_name: &str) -> String {
    format!("{}{}Result", pascal(action_name), resource_name)
}

/// What a mutation's input holds: the attributes the action accepts and its arguments,
/// each with whether it's required. A create requires an accepted attribute that can't be
/// nil and has no default; an update requires none.
fn input_fields(action: &ActionDef, resource: &ResourceDef) -> Vec<(&'static str, AttrType, bool)> {
    let mut fields = Vec::new();
    if matches!(action.kind, ActionKind::Create | ActionKind::Update) {
        for attr in resource.attributes {
            if action.accept.contains(&attr.name) {
                let required = action.kind == ActionKind::Create
                    && !attr.allow_nil
                    && attr.default_fn.is_none()
                    && !attr.generated;
                fields.push((attr.name, attr.ty, required));
            }
        }
    }
    for arg in action.arguments {
        fields.push((arg.name, arg.ty, !arg.allow_nil));
    }
    if matches!(action.kind, ActionKind::Update | ActionKind::Destroy)
        && let Some(version) = resource.optimistic_lock_attribute()
    {
        fields.push((version, AttrType::Integer, false));
    }
    fields
}

/// Registers `<Mutation>Result { result, errors }`. A destroy's result is the record it
/// destroyed.
pub fn register_action_payload(
    builder: SchemaBuilder,
    action: &'static ActionDef,
    resource: &'static ResourceDef,
) -> SchemaBuilder {
    fn payload<'a>(ctx: &ResolverContext<'a>) -> Option<&'a MutationPayload> {
        ctx.parent_value.downcast_ref::<MutationPayload>()
    }
    builder.register(
        Object::new(mutation_payload_name(action.name, resource.name))
            .field(Field::new("result", TypeRef::named(resource.name), |ctx| {
                FieldFuture::new(async move {
                    Ok(payload(&ctx)
                        .and_then(|p| p.result.as_ref())
                        .map(|record| FieldValue::borrowed_any(record)))
                })
            }))
            .field(Field::new(
                "errors",
                TypeRef::named_nn_list_nn(MUTATION_ERROR),
                |ctx| {
                    FieldFuture::new(async move {
                        let errors = payload(&ctx).map(|p| p.errors.as_slice()).unwrap_or_default();
                        Ok(Some(FieldValue::list(errors.iter().map(|e| FieldValue::borrowed_any(e)))))
                    })
                },
            )),
    )
}

/// Registers `<Mutation>Input`, if the action takes any input.
pub fn register_action_input(
    builder: SchemaBuilder,
    action: &'static ActionDef,
    resource: &'static ResourceDef,
) -> SchemaBuilder {
    let fields = input_fields(action, resource);
    if fields.is_empty() {
        return builder;
    }
    let mut input = InputObject::new(mutation_input_name(action.name, resource.name));
    for (name, ty, required) in fields {
        input = input.field(InputValue::new(
            camel(name),
            attr_type_to_type_ref(resource.name, name, ty, !required),
        ));
    }
    builder.register(input)
}

fn failed(err: &AshError) -> Option<FieldValue<'static>> {
    Some(FieldValue::owned_any(MutationPayload {
        result: None,
        errors: vec![UserError::from_ash_error(err)],
    }))
}

fn succeeded(result: Option<FieldMap>) -> Option<FieldValue<'static>> {
    Some(FieldValue::owned_any(MutationPayload {
        result,
        errors: Vec::new(),
    }))
}

/// The mutation for a create, update or destroy: `<action><Resource>(input)` for a create,
/// `<action><Resource>(id: ID!, input)` for an update or destroy, as AshGraphql's are.
pub fn build_action_mutation<D: DataLayer + Clone + 'static>(
    action: &'static ActionDef,
    resource: &'static ResourceDef,
) -> Field {
    let fields = input_fields(action, resource);
    let pk = resource
        .attributes
        .iter()
        .find(|a| a.primary_key)
        .map(|a| a.name)
        .unwrap_or("id");

    let mut field = Field::new(
        mutation_name(action.name, resource.name),
        TypeRef::named_nn(mutation_payload_name(action.name, resource.name)),
        move |ctx| {
            FieldFuture::new(async move {
                let ash = crate::request::request_context::<D>(&ctx)?;
                let ash = &*ash;

                let mut input = FieldMap::new();
                let mut version = None;
                if let Some(given) = ctx.args.get("input").filter(|value| !value.is_null()) {
                    let given = given.object()?;
                    for (name, ty, _) in input_fields(action, resource) {
                        let Some(value) = given.get(&camel(name)) else {
                            continue;
                        };
                        if Some(name) == resource.optimistic_lock_attribute()
                            && action.kind != ActionKind::Create
                            && !action.accept.contains(&name)
                        {
                            version = value.i64().ok();
                            continue;
                        }
                        let value = if value.is_null() { Value::Null } else { parse_input_val(&value, ty)? };
                        input.insert(name.to_string(), value);
                    }
                }

                if action.kind == ActionKind::Create {
                    return Ok(match create_dynamic(ash, resource, action, input).await {
                        Ok(mut stored) => {
                            let _ = redact_fields(resource, ash.actor.as_ref(), &mut stored);
                            succeeded(Some(stored))
                        }
                        Err(e) => failed(&e),
                    });
                }

                let id_arg = ctx
                    .args
                    .get("id")
                    .ok_or_else(|| async_graphql::Error::new("Missing required id argument"))?;
                let id = Uuid::parse_str(id_arg.string()?)
                    .map_err(|e| async_graphql::Error::new(format!("Invalid UUID: {e}")))?;

                // The record as a read would see it, so an archived record or another
                // tenant's is not found.
                let (filter, tenant) = ash_core::visible_scope(
                    resource,
                    Some(Filter::eq(pk, Value::Uuid(id))),
                    ash.tenant.clone(),
                )?;
                let existing = ash
                    .data
                    .run_query(resource, &CompiledQuery { filter, tenant, ..CompiledQuery::default() })
                    .await
                    .ok()
                    .and_then(|records| records.into_iter().next());
                let Some(existing) = existing else {
                    return Ok(failed(&AshError::NotFound));
                };
                if let (Some(expected), Some(v_attr)) = (version, resource.optimistic_lock_attribute()) {
                    let current = existing.get(v_attr).and_then(|v| v.as_int()).unwrap_or(1);
                    if current != expected {
                        return Ok(failed(&AshError::StaleRecord {
                            resource: resource.name,
                            id,
                        }));
                    }
                }

                Ok(match action.kind {
                    ActionKind::Update => match update_dynamic(ash, resource, action, id, input).await {
                        Ok(mut updated) => {
                            let _ = redact_fields(resource, ash.actor.as_ref(), &mut updated);
                            succeeded(Some(updated))
                        }
                        Err(e) => failed(&e),
                    },
                    _ => match destroy_dynamic(ash, resource, action, id, &existing).await {
                        Ok(_) => {
                            let mut destroyed = existing;
                            let _ = redact_fields(resource, ash.actor.as_ref(), &mut destroyed);
                            succeeded(Some(destroyed))
                        }
                        Err(e) => failed(&e),
                    },
                })
            })
        },
    );

    if action.kind != ActionKind::Create {
        field = field.argument(InputValue::new("id", TypeRef::named_nn(TypeRef::ID)));
    }
    if !fields.is_empty() {
        let input = mutation_input_name(action.name, resource.name);
        let required = fields.iter().any(|(_, _, required)| *required);
        field = field.argument(InputValue::new(
            "input",
            if required { TypeRef::named_nn(input) } else { TypeRef::named(input) },
        ));
    }
    field
}
