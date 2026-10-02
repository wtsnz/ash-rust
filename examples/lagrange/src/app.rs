//! Wiring: the fleet database, the telemetry database, and the notifiers every action
//! goes through.

use std::sync::Arc;

use ash_authentication::AuthStrategy;
use ash_core::{Actor, Context, DataLayer, Notifier, Resource, Result, StoreRegistry};
use ash_mailer::{Email, EmailNotifier, Mailer};
use ash_pubsub::{PubSub, PubSubNotifier};
use ash_sqlite::Sqlite;
use uuid::Uuid;

use crate::telemetry::{AuditNotifier, TelemetryStore};
use crate::{Contract, CrewMember, Transponder};

/// The address delivery notices come from.
pub const DISPATCH_FROM: &str = "dispatch@lagrange.example";

/// The running platform: one fleet database `D` (memory, SQLite or Postgres) and a
/// SQLite telemetry database.
pub struct Lagrange<D> {
    fleet: D,
    telemetry: Sqlite,
    pubsub: PubSub,
    notifiers: Vec<Arc<dyn Notifier>>,
    base: Context<D>,
}

impl<D> Lagrange<D>
where
    D: DataLayer + Clone + Send + Sync + 'static,
{
    /// Every action on the fleet context is published to `pubsub`, written to the
    /// audit log in the telemetry database, and, for deliveries, emailed to the shipper.
    pub fn new(fleet: D, telemetry: Sqlite, mailer: impl Mailer) -> Self {
        let pubsub = PubSub::new();
        let audit = AuditNotifier::new(Context::new(Self::registry_for(
            fleet.clone(),
            telemetry.clone(),
        )));
        let notifiers: Vec<Arc<dyn Notifier>> = vec![
            Arc::new(PubSubNotifier::new(Arc::new(pubsub.clone()))),
            Arc::new(audit),
            Arc::new(delivery_notices(mailer)),
        ];
        let base = attach(&notifiers, Context::new(fleet.clone()));
        Self {
            fleet,
            telemetry,
            pubsub,
            notifiers,
            base,
        }
    }

    /// A context on both databases, with the same notifiers as the fleet context. The
    /// GraphQL API runs on it.
    pub fn registry_context(&self) -> Context<StoreRegistry> {
        attach(&self.notifiers, Context::new(self.registry()))
    }

    fn registry_for(fleet: D, telemetry: Sqlite) -> StoreRegistry {
        StoreRegistry::new()
            .with_default(fleet)
            .with_store::<TelemetryStore, _>(telemetry)
    }

    /// Both databases behind one data layer: telemetry resources go to the telemetry
    /// database and everything else to the fleet database.
    pub fn registry(&self) -> StoreRegistry {
        Self::registry_for(self.fleet.clone(), self.telemetry.clone())
    }

    pub fn fleet(&self) -> &D {
        &self.fleet
    }

    pub fn telemetry(&self) -> &Sqlite {
        &self.telemetry
    }

    pub fn pubsub(&self) -> &PubSub {
        &self.pubsub
    }

    /// The fleet context with no one signed in.
    pub fn anonymous(&self) -> Context<D> {
        self.base.clone()
    }

    /// The port authority, which runs the shared map and bypasses line policies.
    pub fn port_authority(&self) -> Context<D> {
        self.base
            .clone()
            .with_actor(Actor::new(Uuid::nil()).with_role("port_authority"))
    }

    /// Acts as `crew` within their line.
    pub fn as_crew(&self, crew: &CrewMember) -> Context<D> {
        let actor = CrewMember::auth_strategy().to_actor(crew);
        self.base
            .clone()
            .with_actor(actor)
            .with_tenant(crew.line.clone())
    }

    /// The crew member with this email and password.
    pub async fn authenticate(&self, email: &str, password: &str) -> Result<CrewMember> {
        CrewMember::auth_strategy()
            .sign_in_with_password(&self.base, email, password)
            .await
            .map_err(|_| ash_core::Error::Forbidden)
    }

    /// Signs in with an email and password.
    pub async fn sign_in(&self, email: &str, password: &str) -> Result<Context<D>> {
        let crew = self.authenticate(email, password).await?;
        Ok(self.as_crew(&crew))
    }

    /// A transponder signs in with its API key. Its context writes telemetry through the
    /// registry, so pings land in the telemetry database.
    pub async fn transponder(&self, api_key: &str) -> Result<Context<StoreRegistry>> {
        let strategy: AuthStrategy<Transponder> = Transponder::auth_strategy();
        let transponder = strategy
            .authenticate_api_key(&self.base, api_key)
            .await
            .map_err(|_| ash_core::Error::Forbidden)?;
        Ok(Context::new(self.registry())
            .with_actor(strategy.to_actor(&transponder))
            .with_tenant(transponder.line.clone()))
    }
}

fn attach<T>(notifiers: &[Arc<dyn Notifier>], ctx: Context<T>) -> Context<T> {
    notifiers
        .iter()
        .fold(ctx, |ctx, notifier| ctx.with_notifier(Arc::clone(notifier)))
}

/// Emails the shipper when their contract is delivered.
fn delivery_notices(mailer: impl Mailer) -> EmailNotifier {
    EmailNotifier::new(mailer).on_action(Contract::DEF.name, "deliver", |notification| {
        let field = |name: &str| match notification.record_fields.get(name) {
            Some(ash_core::Value::String(value)) => value.clone(),
            _ => String::new(),
        };
        Email::new()
            .from(DISPATCH_FROM)
            .to(field("shipper_email"))
            .subject(format!("Delivered: {}", field("external_ref")))
            .text(format!(
                "Your cargo under booking {} has arrived and is ready for collection.",
                field("external_ref")
            ))
    })
}
