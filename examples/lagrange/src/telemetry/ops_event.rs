use std::future::Future;
use std::pin::Pin;

use ash_core::{
    Context, DataLayer, Inet, Notification, Notifier, Result, UtcDateTime, Value, resource,
};
use uuid::Uuid;

use super::TelemetryStore;

resource! {
    /// One committed fleet action, written to the telemetry store by [`AuditNotifier`].
    OpsEvent {
        table "ops_events";
        store TelemetryStore;

        attributes {
            id: Uuid [pk];
            line: Option<String>;
            actor_id: Option<Uuid>;
            resource: String;
            action: String;
            record_id: Uuid;
            client_ip: Option<Inet>;
            at: UtcDateTime;
        }

        indexes {
            index by_record: [record_id];
            index by_time: [at], using: brin;
        }

        actions {
            create record {
                primary;
                accept [line, actor_id, resource, action, record_id, client_ip, at];
            }

            read read {
                primary;
            }
        }
    }
}

/// Writes an [`OpsEvent`] for every committed fleet action. Notifiers run after the
/// transaction commits, so an action that rolls back leaves no audit entry.
pub struct AuditNotifier<D> {
    store: Context<D>,
}

impl<D> AuditNotifier<D> {
    pub fn new(store: Context<D>) -> Self {
        Self { store }
    }
}

impl<D> std::fmt::Debug for AuditNotifier<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditNotifier").finish_non_exhaustive()
    }
}

impl<D: DataLayer + Send + Sync + 'static> Notifier for AuditNotifier<D> {
    fn notify<'a>(
        &'a self,
        notification: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let client_ip = match notification.metadata.get("client_ip") {
                Some(Value::String(ip)) => Some(Inet::parse(ip)?),
                _ => None,
            };
            OpsEvent::record(&self.store)
                .line(notification.tenant.clone())
                .actor_id(notification.actor.as_ref().map(|actor| actor.id))
                .resource(notification.resource)
                .action(notification.action.clone())
                .record_id(notification.id)
                .client_ip(client_ip)
                .at(UtcDateTime::parse(&ash_core::utc_now_iso8601())?)
                .await?;
            Ok(())
        })
    }
}
