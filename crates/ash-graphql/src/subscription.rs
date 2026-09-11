use ash_core::redact_fields;
use ash_core::{ActionKind, Actor, ResourceDef};

use crate::read_scope::record_visible_for_read;
use ash_pubsub::PubSub;
use async_graphql::dynamic::*;
use uuid::Uuid;

use crate::filter::{parse_resource_filter, resource_filter_input_name};

/// Returns camelCase name of the resource (e.g. "ticket" for "Ticket").
pub fn uncapitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
    }
}

pub fn build_resource_subscriptions(
    resource: &'static ResourceDef,
    pubsub: PubSub,
) -> Vec<SubscriptionField> {
    let mut fields = Vec::new();
    let lower_res = uncapitalize(resource.name);

    // 1. <resource>Created(filter: <Resource>FilterInput): <Resource>!
    let created_field_name = format!("{lower_res}Created");
    let pubsub_clone = pubsub.clone();
    let mut created_sub = SubscriptionField::new(
        created_field_name,
        TypeRef::named_nn(resource.name),
        move |ctx| {
            let pubsub = pubsub_clone.clone();
            SubscriptionFieldFuture::new(async move {
                let filter_arg = ctx.args.get("filter");
                let filter = if let Some(f_acc) = filter_arg {
                    let obj = f_acc.object()?;
                    Some(parse_resource_filter(resource, &obj)?)
                } else {
                    None
                };

                let mut sub = pubsub.subscribe(format!("{}:*", resource.name.to_lowercase()));
                let actor = ctx.data_opt::<Actor>().cloned();

                let stream = async_stream::stream! {
                    while let Ok(notif) = sub.recv().await {
                        if notif.action_kind == ActionKind::Create {
                            let mut record = notif.record_fields;
                            if let Some(f) = &filter
                                && !f.matches(&record)
                            {
                                continue;
                            }
                            if !record_visible_for_read(resource, actor.as_ref(), &record) {
                                continue;
                            }
                            let _ = redact_fields(resource, actor.as_ref(), &mut record);
                            yield Ok(FieldValue::owned_any(record));
                        }
                    }
                };

                Ok(stream)
            })
        },
    );
    created_sub = created_sub.argument(InputValue::new(
        "filter",
        TypeRef::named(resource_filter_input_name(resource.name)),
    ));
    fields.push(created_sub);

    // 2. <resource>Updated(id: ID): <Resource>!
    let updated_field_name = format!("{lower_res}Updated");
    let pubsub_clone2 = pubsub.clone();
    let mut updated_sub = SubscriptionField::new(
        updated_field_name,
        TypeRef::named_nn(resource.name),
        move |ctx| {
            let pubsub = pubsub_clone2.clone();
            SubscriptionFieldFuture::new(async move {
                let target_id = if let Some(id_arg) = ctx.args.get("id") {
                    let s = id_arg.string()?;
                    Some(
                        Uuid::parse_str(s)
                            .map_err(|e| async_graphql::Error::new(format!("Invalid UUID: {e}")))?,
                    )
                } else {
                    None
                };

                let mut sub = pubsub.subscribe(format!("{}:*", resource.name.to_lowercase()));
                let actor = ctx.data_opt::<Actor>().cloned();

                let stream = async_stream::stream! {
                    while let Ok(notif) = sub.recv().await {
                        if notif.action_kind == ActionKind::Update {
                            if let Some(tid) = target_id
                                && notif.id != tid
                            {
                                continue;
                            }
                            let mut record = notif.record_fields;
                            if !record_visible_for_read(resource, actor.as_ref(), &record) {
                                continue;
                            }
                            let _ = redact_fields(resource, actor.as_ref(), &mut record);
                            yield Ok(FieldValue::owned_any(record));
                        }
                    }
                };

                Ok(stream)
            })
        },
    );
    updated_sub = updated_sub.argument(InputValue::new("id", TypeRef::named(TypeRef::ID)));
    fields.push(updated_sub);

    // 3. <resource>Destroyed(id: ID): ID!
    let destroyed_field_name = format!("{lower_res}Destroyed");
    let mut destroyed_sub = SubscriptionField::new(
        destroyed_field_name,
        TypeRef::named_nn(TypeRef::ID),
        move |ctx| {
            let pubsub = pubsub.clone();
            SubscriptionFieldFuture::new(async move {
                let target_id = if let Some(id_arg) = ctx.args.get("id") {
                    let s = id_arg.string()?;
                    Some(
                        Uuid::parse_str(s)
                            .map_err(|e| async_graphql::Error::new(format!("Invalid UUID: {e}")))?,
                    )
                } else {
                    None
                };

                let mut sub = pubsub.subscribe(format!("{}:*", resource.name.to_lowercase()));
                let actor = ctx.data_opt::<Actor>().cloned();

                let stream = async_stream::stream! {
                    while let Ok(notif) = sub.recv().await {
                        if notif.action_kind == ActionKind::Destroy {
                            if let Some(tid) = target_id
                                && notif.id != tid
                            {
                                continue;
                            }
                            if !record_visible_for_read(resource, actor.as_ref(), &notif.record_fields) {
                                continue;
                            }
                            yield Ok(FieldValue::value(async_graphql::Value::String(notif.id.to_string())));
                        }
                    }
                };

                Ok(stream)
            })
        },
    );
    destroyed_sub = destroyed_sub.argument(InputValue::new("id", TypeRef::named(TypeRef::ID)));
    fields.push(destroyed_sub);

    fields
}
