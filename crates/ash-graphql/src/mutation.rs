use ash_core::create_dynamic;
use ash_core::destroy_dynamic;
use ash_core::redact_fields;
use ash_core::update_dynamic;
use ash_core::{ActionDef, ActionKind, AttrType, CompiledQuery, Context, DataLayer, Error as AshError, FieldMap, Filter, Notification, ResourceDef, Value};
use async_graphql::dynamic::*;
use async_graphql::Value as GqlValue;
use uuid::Uuid;

use crate::error::UserError;
use crate::types::attr_type_to_type_ref;

#[derive(Clone)]
pub struct MutationPayload {
    pub result: Option<FieldMap>,
    pub errors: Vec<UserError>,
    pub success: bool,
}

pub fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

pub fn mutation_name(action_name: &str, resource_name: &str) -> String {
    format!("{}{}", action_name, resource_name)
}

pub fn mutation_input_name(action_name: &str, resource_name: &str) -> String {
    format!("{}{}Input", capitalize_first(action_name), resource_name)
}

pub fn mutation_payload_name(action_name: &str, resource_name: &str) -> String {
    format!("{}{}Payload", capitalize_first(action_name), resource_name)
}

/// Registers mutation payload object type for an action.
pub fn register_action_payload(
    builder: SchemaBuilder,
    action: &'static ActionDef,
    resource: &'static ResourceDef,
) -> SchemaBuilder {
    let payload_name = mutation_payload_name(action.name, resource.name);
    let res_name = resource.name;

    let payload_obj = Object::new(payload_name)
        .field(Field::new(
            "result",
            TypeRef::named(res_name),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(payload) = ctx.parent_value.downcast_ref::<MutationPayload>() {
                        match &payload.result {
                            Some(res) => Ok(Some(FieldValue::owned_any(res.clone()))),
                            None => Ok(None),
                        }
                    } else {
                        Ok(None)
                    }
                })
            },
        ))
        .field(Field::new(
            "errors",
            TypeRef::named_nn_list_nn("UserError"),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(payload) = ctx.parent_value.downcast_ref::<MutationPayload>() {
                        Ok(Some(FieldValue::list(
                            payload.errors.iter().cloned().map(FieldValue::owned_any),
                        )))
                    } else {
                        Ok(Some(FieldValue::list(Vec::<FieldValue>::new())))
                    }
                })
            },
        ))
        .field(Field::new(
            "success",
            TypeRef::named_nn(TypeRef::BOOLEAN),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(payload) = ctx.parent_value.downcast_ref::<MutationPayload>() {
                        Ok(Some(FieldValue::value(GqlValue::Boolean(payload.success))))
                    } else {
                        Ok(Some(FieldValue::value(GqlValue::Boolean(false))))
                    }
                })
            },
        ));

    builder.register(payload_obj)
}

/// Registers mutation input object type for an action.
pub fn register_action_input(
    builder: SchemaBuilder,
    action: &'static ActionDef,
    resource: &'static ResourceDef,
) -> SchemaBuilder {
    let input_name = mutation_input_name(action.name, resource.name);
    let mut input_obj = InputObject::new(input_name);

    // 1. If Update or Destroy, require ID
    if matches!(action.kind, ActionKind::Update | ActionKind::Destroy) {
        input_obj = input_obj.field(InputValue::new("id", TypeRef::named_nn(TypeRef::ID)));
        if resource.optimistic_lock_attribute().is_some() {
            input_obj = input_obj.field(InputValue::new("version", TypeRef::named(TypeRef::INT)));
        }
    }

    // 2. Attributes accepted by the action
    if matches!(action.kind, ActionKind::Create | ActionKind::Update) {
        for attr in resource.attributes {
            let is_accepted = if !action.accept.is_empty() {
                action.accept.contains(&attr.name)
            } else {
                !attr.primary_key && !attr.generated && !attr.version
            };

            if is_accepted {
                let type_ref = attr_type_to_type_ref(resource.name, attr.name, attr.ty, true);
                input_obj = input_obj.field(InputValue::new(attr.name, type_ref));
            }
        }
    }

    // 3. Action arguments
    for arg in action.arguments {
        let type_ref = attr_type_to_type_ref(resource.name, arg.name, arg.ty, arg.allow_nil);
        input_obj = input_obj.field(InputValue::new(arg.name, type_ref));
    }

    builder.register(input_obj)
}

/// Builds the GraphQL mutation [`Field`] for a resource action.
pub fn build_action_mutation<D: DataLayer + Clone + 'static>(
    action: &'static ActionDef,
    resource: &'static ResourceDef,
) -> Field {
    let m_name = mutation_name(action.name, resource.name);
    let input_name = mutation_input_name(action.name, resource.name);
    let payload_name = mutation_payload_name(action.name, resource.name);

    Field::new(m_name, TypeRef::named_nn(payload_name), move |ctx| {
        FieldFuture::new(async move {
            let ctx_ash = ctx.data::<Context<D>>()?;

            let input_arg = ctx
                .args
                .get("input")
                .ok_or_else(|| async_graphql::Error::new("Missing required input object"))?;
            let input_obj = input_arg.object()?;

            let mut input_map = FieldMap::new();

            // Extract accepted attributes
            for attr in resource.attributes {
                if let Some(val) = input_obj.get(attr.name) {
                    let ash_val = parse_input_val(&val, attr.ty)?;
                    if !ash_val.is_null() {
                        input_map.insert(attr.name.to_string(), ash_val);
                    }
                }
            }

            // Extract action arguments
            for arg in action.arguments {
                if let Some(val) = input_obj.get(arg.name) {
                    let ash_val = parse_input_val(&val, arg.ty)?;
                    if !ash_val.is_null() {
                        input_map.insert(arg.name.to_string(), ash_val);
                    }
                }
            }

            match action.kind {
                ActionKind::Create => {
                    match create_dynamic(ctx_ash, resource, action, input_map).await {
                        Ok(mut stored) => {
                            if let Some(pubsub) = ctx.data_opt::<ash_pubsub::PubSub>() {
                                let rec_id = stored.get("id").and_then(|v| v.as_uuid()).unwrap_or_else(Uuid::new_v4);
                                let notif = Notification::new(
                                    resource.name,
                                    action.name,
                                    ActionKind::Create,
                                    rec_id,
                                    stored.clone(),
                                    None,
                                    ctx_ash.actor.clone(),
                                    FieldMap::new(),
                                );
                                let topic = format!("{}:{}", resource.name.to_lowercase(), action.name);
                                pubsub.publish(&topic, notif.clone());
                                pubsub.publish(&format!("{}:*", resource.name.to_lowercase()), notif);
                            }

                            let _ = redact_fields(resource, ctx_ash.actor.as_ref(), &mut stored);
                            Ok(Some(FieldValue::owned_any(MutationPayload {
                                result: Some(stored),
                                errors: Vec::new(),
                                success: true,
                            })))
                        }
                        Err(e) => Ok(Some(FieldValue::owned_any(MutationPayload {
                            result: None,
                            errors: vec![UserError::from_ash_error(&e)],
                            success: false,
                        }))),
                    }
                }
                ActionKind::Update => {
                    let id_val = input_obj
                        .get("id")
                        .ok_or_else(|| async_graphql::Error::new("Missing required id field in input"))?;
                    let id_str = id_val.string()?;
                    let id = Uuid::parse_str(id_str)
                        .map_err(|e| async_graphql::Error::new(format!("Invalid UUID: {e}")))?;

                    // Check optimistic lock if client sent a version
                    if let Some(client_version) = input_obj.get("version")
                        && let Some(v_attr) = resource.optimistic_lock_attribute()
                    {
                        let expected_v = client_version.i64()?;
                        let pk = resource
                            .attributes
                            .iter()
                            .find(|a| a.primary_key)
                            .map(|a| a.name)
                            .unwrap_or("id");

                        let query = CompiledQuery {
                            filter: Some(Filter::eq(pk, Value::Uuid(id))),
                            tenant: ctx_ash.tenant.clone(),
                            ..CompiledQuery::default()
                        };

                        if let Ok(records) = ctx_ash.data.run_query(resource, &query).await
                            && let Some(existing) = records.first()
                        {
                            let current_v = existing
                                .get(v_attr)
                                .and_then(|v| v.as_int())
                                .unwrap_or(1);
                            if current_v != expected_v {
                                return Ok(Some(FieldValue::owned_any(MutationPayload {
                                    result: None,
                                    errors: vec![UserError::from_ash_error(&AshError::StaleRecord {
                                        resource: resource.name,
                                        id,
                                    })],
                                    success: false,
                                })));
                            }
                        }
                    }

                    match update_dynamic(ctx_ash, resource, action, id, input_map).await {
                        Ok(mut updated) => {
                            if let Some(pubsub) = ctx.data_opt::<ash_pubsub::PubSub>() {
                                let notif = Notification::new(
                                    resource.name,
                                    action.name,
                                    ActionKind::Update,
                                    id,
                                    updated.clone(),
                                    None,
                                    ctx_ash.actor.clone(),
                                    FieldMap::new(),
                                );
                                let topic = format!("{}:{}", resource.name.to_lowercase(), action.name);
                                pubsub.publish(&topic, notif.clone());
                                pubsub.publish(&format!("{}:*", resource.name.to_lowercase()), notif);
                            }

                            let _ = redact_fields(resource, ctx_ash.actor.as_ref(), &mut updated);
                            Ok(Some(FieldValue::owned_any(MutationPayload {
                                result: Some(updated),
                                errors: Vec::new(),
                                success: true,
                            })))
                        }
                        Err(e) => Ok(Some(FieldValue::owned_any(MutationPayload {
                            result: None,
                            errors: vec![UserError::from_ash_error(&e)],
                            success: false,
                        }))),
                    }
                }
                ActionKind::Destroy => {
                    let id_val = input_obj
                        .get("id")
                        .ok_or_else(|| async_graphql::Error::new("Missing required id field in input"))?;
                    let id_str = id_val.string()?;
                    let id = Uuid::parse_str(id_str)
                        .map_err(|e| async_graphql::Error::new(format!("Invalid UUID: {e}")))?;

                    let pk = resource
                        .attributes
                        .iter()
                        .find(|a| a.primary_key)
                        .map(|a| a.name)
                        .unwrap_or("id");

                    let query = CompiledQuery {
                        filter: Some(Filter::eq(pk, Value::Uuid(id))),
                        tenant: ctx_ash.tenant.clone(),
                        ..CompiledQuery::default()
                    };

                    let existing_records = ctx_ash.data.run_query(resource, &query).await;
                    let existing_field_map = match existing_records {
                        Ok(records) if !records.is_empty() => records.into_iter().next().unwrap(),
                        _ => {
                            return Ok(Some(FieldValue::owned_any(MutationPayload {
                                result: None,
                                errors: vec![UserError::from_ash_error(&AshError::NotFound)],
                                success: false,
                            })));
                        }
                    };

                    // Check optimistic locking if provided
                    if let Some(client_version) = input_obj.get("version")
                        && let Some(v_attr) = resource.optimistic_lock_attribute()
                    {
                        let expected_v = client_version.i64()?;
                        let current_v = existing_field_map
                            .get(v_attr)
                            .and_then(|v| v.as_int())
                            .unwrap_or(1);
                        if current_v != expected_v {
                            return Ok(Some(FieldValue::owned_any(MutationPayload {
                                result: None,
                                errors: vec![UserError::from_ash_error(&AshError::StaleRecord {
                                    resource: resource.name,
                                    id,
                                })],
                                success: false,
                            })));
                        }
                    }

                    match destroy_dynamic(ctx_ash, resource, action, id, &existing_field_map).await {
                        Ok(_) => {
                            if let Some(pubsub) = ctx.data_opt::<ash_pubsub::PubSub>() {
                                let notif = Notification::new(
                                    resource.name,
                                    action.name,
                                    ActionKind::Destroy,
                                    id,
                                    existing_field_map.clone(),
                                    Some(existing_field_map),
                                    ctx_ash.actor.clone(),
                                    FieldMap::new(),
                                );
                                let topic = format!("{}:{}", resource.name.to_lowercase(), action.name);
                                pubsub.publish(&topic, notif.clone());
                                pubsub.publish(&format!("{}:*", resource.name.to_lowercase()), notif);
                            }

                            Ok(Some(FieldValue::owned_any(MutationPayload {
                                result: None,
                                errors: Vec::new(),
                                success: true,
                            })))
                        }
                        Err(e) => Ok(Some(FieldValue::owned_any(MutationPayload {
                            result: None,
                            errors: vec![UserError::from_ash_error(&e)],
                            success: false,
                        }))),
                    }
                }
                _ => Ok(Some(FieldValue::owned_any(MutationPayload {
                    result: None,
                    errors: vec![UserError {
                        message: "Unsupported mutation action kind".into(),
                        field: None,
                        code: "UNSUPPORTED_ACTION".into(),
                    }],
                    success: false,
                }))),
            }
        })
    })
    .argument(InputValue::new("input", TypeRef::named_nn(input_name)))
}

fn parse_input_val(
    acc: &ValueAccessor<'_>,
    ty: AttrType,
) -> Result<Value, async_graphql::Error> {
    match ty {
        AttrType::Uuid => {
            let s = acc.string()?;
            let u = uuid::Uuid::parse_str(s)
                .map_err(|e| async_graphql::Error::new(format!("Invalid UUID: {e}")))?;
            Ok(Value::Uuid(u))
        }
        AttrType::String => {
            let s = acc.string()?;
            Ok(Value::String(s.to_string()))
        }
        AttrType::Integer => {
            let n = acc.i64()?;
            Ok(Value::Int(n))
        }
        AttrType::Boolean => {
            let b = acc.boolean()?;
            Ok(Value::Bool(b))
        }
        AttrType::Atom { one_of } => {
            let name = acc.enum_name()?;
            if let Some(matched) = one_of.iter().find(|&&s| s.eq_ignore_ascii_case(name)) {
                Ok(Value::String((*matched).to_string()))
            } else {
                Ok(Value::String(name.to_string()))
            }
        }
        _ => Ok(Value::Null),
    }
}
