use ash_core::{ActionKind, Actor, Context, DataLayer, ResourceDef, record_visible};

use crate::redact::redact_record;
use crate::request::request_actor;
use ash_pubsub::PubSub;
use async_graphql::ErrorExtensions;
use async_graphql::dynamic::*;
use tokio::sync::broadcast::error::RecvError;

use crate::filter::{parse_resource_filter, resource_filter_input_name};

/// Returns camelCase name of the resource (e.g. "ticket" for "Ticket").
pub fn uncapitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
    }
}

/// The next event for a subscription, or `None` once the pubsub has shut down.
///
/// The pubsub buffers a bounded number of events per topic, so one slow subscriber can't
/// grow the server's memory without limit. A subscriber that falls further behind than
/// that misses the oldest events. It hears so as an error with code `MISSED_EVENTS` and
/// the number missed, and the subscription ends there (async-graphql ends a subscription
/// after any error). A client holding state built from events resubscribes and re-reads
/// it, as the generated TypeScript client does.
async fn next_event(
    sub: &mut ash_pubsub::Subscription,
) -> Option<Result<std::sync::Arc<ash_core::Notification>, async_graphql::Error>> {
    match sub.recv().await {
        Ok(notification) => Some(Ok(notification)),
        Err(RecvError::Lagged(missed)) => Some(Err(async_graphql::Error::new(format!(
            "Missed {missed} events: the subscriber fell behind"
        ))
        .extend_with(|_, extensions| {
            extensions.set("code", "MISSED_EVENTS");
            extensions.set("missed", missed);
        }))),
        Err(RecvError::Closed) => None,
    }
}

/// The subscriber's actor and tenant, as the request's other resolvers see them.
fn subscriber<D: DataLayer + 'static>(
    ctx: &ResolverContext<'_>,
) -> (Option<Actor>, Option<String>) {
    let tenant = ctx
        .ctx
        .data_opt::<Context<D>>()
        .and_then(|context| context.tenant.clone());
    (request_actor::<D>(ctx).cloned(), tenant)
}

/// What a subscription's payload carries: the record created or updated, or the id of
/// the one destroyed.
#[derive(Clone)]
enum Event {
    Record(ash_core::FieldMap),
    Id(String),
}

/// `cabCreated` → `cab_created_result`, as AshGraphql names a subscription's payload.
pub fn subscription_result_name(field: &str) -> String {
    format!("{}_result", crate::names::snake(field))
}

/// The three subscriptions a resource has, as AshGraphql declares them with
/// `action_types`: `<resource>Created`, `Updated` and `Destroyed`, each kind and the key
/// its payload carries the record under.
fn kinds(resource: &ResourceDef) -> [(String, ActionKind, &'static str); 3] {
    let lower = uncapitalize(resource.name);
    [
        (format!("{lower}Created"), ActionKind::Create, "created"),
        (format!("{lower}Updated"), ActionKind::Update, "updated"),
        (format!("{lower}Destroyed"), ActionKind::Destroy, "destroyed"),
    ]
}

/// Registers each subscription's payload type: `<name>_result { created: <Resource> }`,
/// `{ updated: <Resource> }` or `{ destroyed: ID }`.
pub fn register_subscription_results(
    mut builder: SchemaBuilder,
    resource: &'static ResourceDef,
) -> SchemaBuilder {
    for (field, kind, key) in kinds(resource) {
        let type_ref = if kind == ActionKind::Destroy {
            TypeRef::named(TypeRef::ID)
        } else {
            TypeRef::named(resource.name)
        };
        builder = builder.register(Object::new(subscription_result_name(&field)).field(Field::new(
            key,
            type_ref,
            |ctx| {
                FieldFuture::new(async move {
                    Ok(match ctx.parent_value.downcast_ref::<Event>() {
                        // Borrowed: the event lives as long as its resolution.
                        Some(Event::Record(record)) => Some(FieldValue::borrowed_any(record)),
                        Some(Event::Id(id)) => Some(FieldValue::value(async_graphql::Value::String(id.clone()))),
                        None => None,
                    })
                })
            },
        )));
    }
    builder
}

/// `<resource>Created(filter)`, `<resource>Updated(filter)` and
/// `<resource>Destroyed(filter)`: every change of that kind to a record the subscriber
/// can read, in its tenant, that matches the filter.
pub fn build_resource_subscriptions<D: DataLayer + 'static>(
    resource: &'static ResourceDef,
    pubsub: PubSub,
) -> Vec<SubscriptionField> {
    kinds(resource)
        .into_iter()
        .map(|(field, kind, _)| {
            let pubsub = pubsub.clone();
            SubscriptionField::new(
                field.clone(),
                TypeRef::named(subscription_result_name(&field)),
                move |ctx| {
                    let pubsub = pubsub.clone();
                    SubscriptionFieldFuture::new(async move {
                        // An argument given as `null`, as an unset variable is, means none.
                        let filter = match ctx.args.get("filter").filter(|arg| !arg.is_null()) {
                            Some(filter) => Some(parse_resource_filter(resource, &filter.object()?)?),
                            None => None,
                        };
                        let mut sub = pubsub.subscribe(format!("{}:*", resource.name.to_lowercase()));
                        let (actor, tenant) = subscriber::<D>(&ctx);

                        let stream = async_stream::stream! {
                            while let Some(event) = next_event(&mut sub).await {
                                let notif = match event {
                                    Ok(notif) => notif,
                                    Err(missed) => {
                                        yield Err(missed);
                                        break;
                                    }
                                };
                                if notif.action_kind != kind {
                                    continue;
                                }
                                if let Some(f) = &filter
                                    && !f.matches_on(resource, &notif.record_fields)
                                {
                                    continue;
                                }
                                if !record_visible(resource, actor.as_ref(), tenant.as_deref(), &notif.record_fields) {
                                    continue;
                                }
                                let event = if kind == ActionKind::Destroy {
                                    Event::Id(notif.id.to_string())
                                } else {
                                    // Shared with every other subscriber: copy only what this one sends.
                                    let mut record = notif.record_fields.clone();
                                    redact_record(resource, actor.as_ref(), &mut record);
                                    Event::Record(record)
                                };
                                yield Ok(FieldValue::owned_any(event));
                            }
                        };
                        Ok(stream)
                    })
                },
            )
            .argument(InputValue::new(
                "filter",
                TypeRef::named(resource_filter_input_name(resource.name)),
            ))
        })
        .collect()
}
