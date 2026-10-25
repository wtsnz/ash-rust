//! Wiring: the fleet database, the telemetry database, and the notifiers every action
//! goes through.

use std::sync::Arc;

use ash_authentication::AuthStrategy;
use ash_core::{Context, DataLayer, Notifier, Resource, Result, Runtime, StoreRegistry};
use ash_mailer::{Email, EmailNotifier, Mailer};
use ash_pubsub::{PubSub, PubSubNotifier};
use ash_sqlite::Sqlite;

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
    base: Runtime<D>,
}

impl<D> Lagrange<D>
where
    D: DataLayer + Clone + Send + Sync + 'static,
{
    /// Every action on the fleet context is published to `pubsub`, written to the
    /// audit log in the telemetry database, and, for deliveries, emailed to the shipper.
    pub fn new(fleet: D, telemetry: Sqlite, mailer: impl Mailer) -> Self {
        let pubsub = PubSub::new();
        let audit = AuditNotifier::new(
            ash_core::Runtime::new(Self::registry_for(fleet.clone(), telemetry.clone()))
                .bootstrap(),
        );
        let notifiers: Vec<Arc<dyn Notifier>> = vec![
            Arc::new(PubSubNotifier::new(Arc::new(pubsub.clone()))),
            Arc::new(audit),
            Arc::new(delivery_notices(mailer)),
        ];
        let base = attach(&notifiers, Runtime::new(fleet.clone()));
        Self {
            fleet,
            telemetry,
            pubsub,
            base,
        }
    }

    /// A context on both databases, with the same notifiers as the fleet context. The
    /// GraphQL API runs on it.
    pub fn registry_context(&self) -> Context<StoreRegistry> {
        self.registry_runtime().bootstrap()
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
        self.base.bootstrap()
    }

    /// The port authority, which runs the shared map and bypasses line policies.
    pub fn port_authority(&self) -> Context<D> {
        self.base.system().bind(
            Some(ash_core::Actor::new(uuid::Uuid::nil()).with_role("port_authority")),
            None,
        )
    }

    /// Trusted fleet authority used by seeds and multi-scope workflows.
    pub fn runtime(&self) -> &Runtime<D> {
        &self.base
    }

    pub fn registry_runtime(&self) -> Runtime<StoreRegistry> {
        self.base.with_backend(self.registry())
    }

    /// Acts as `crew` within their line.
    pub fn as_crew(&self, crew: &CrewMember) -> Context<D> {
        let actor = CrewMember::auth_strategy(&self.base).to_actor(crew);
        self.base.bind(Some(actor), Some(crew.line.clone()))
    }

    /// The crew member with this email and password in the adapter-selected line.
    pub async fn authenticate(
        &self,
        line: &str,
        email: &str,
        password: &str,
    ) -> Result<CrewMember> {
        CrewMember::auth_strategy(&self.base)
            .sign_in_with_password(&self.base.bind(None, Some(line.into())), email, password)
            .await
            .map_err(|_| ash_core::Error::Forbidden)
    }

    /// Signs in to a line with an email and password.
    pub async fn sign_in(&self, line: &str, email: &str, password: &str) -> Result<Context<D>> {
        let crew = self.authenticate(line, email, password).await?;
        Ok(self.as_crew(&crew))
    }

    /// A transponder signs in with its API key. Its context writes telemetry through the
    /// registry, so pings land in the telemetry database.
    pub async fn transponder(&self, line: &str, api_key: &str) -> Result<Context<StoreRegistry>> {
        let strategy: AuthStrategy<Transponder> = Transponder::auth_strategy(&self.base);
        let transponder = strategy
            .authenticate_api_key(&self.base.bind(None, Some(line.into())), api_key)
            .await
            .map_err(|_| ash_core::Error::Forbidden)?;
        Ok(self.registry_runtime().bind(
            Some(strategy.to_actor(&transponder)),
            Some(transponder.line.clone()),
        ))
    }
}

fn attach<T>(notifiers: &[Arc<dyn Notifier>], ctx: Runtime<T>) -> Runtime<T> {
    use ash_core::{AttrType, MetadataDef};
    let mut ctx = ctx.with_notifier(Arc::clone(&notifiers[0]));
    // Declare the log's exact disclosure separately for each resource and Runtime.
    for resource in crate::World::<T>::DEF
        .resources
        .iter()
        .chain(crate::Fleet::<T>::DEF.resources)
        .chain(crate::Cargo::<T>::DEF.resources)
    {
        let grant = ctx
            .notifier_grant_for(resource, &[])
            .expect("declared resource")
            .with_actor_id()
            .with_metadata(&[MetadataDef::new("client_ip", AttrType::String)]);
        ctx = ctx.with_notifier(grant.attach(Arc::clone(&notifiers[1])));
    }
    let mail = ctx
        .notifier_grant::<Contract>(&["shipper_email", "external_ref"])
        .expect("declared mail fields");
    ctx.with_notifier(mail.attach(Arc::clone(&notifiers[2])))
}

/// Emails the shipper when their contract is delivered.
fn delivery_notices(mailer: impl Mailer) -> EmailNotifier {
    EmailNotifier::new(mailer).on_action(Contract::DEF.name, "deliver", |notification| {
        let field = |name: &str| match notification.record.get(name) {
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
