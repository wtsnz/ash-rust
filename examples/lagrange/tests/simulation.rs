//! The week-long simulation runs to the end on every backend.

mod support;

use ash_mailer::MemoryMailer;
use lagrange::sim::{Summary, week_in_sol};

async fn the_week_runs<D: support::FleetDb>(app: lagrange::Lagrange<D>, mailer: MemoryMailer) {
    let (summary, log) = week_in_sol(&app).await.unwrap_or_else(|err| panic!("the week failed: {err}"));
    assert_eq!(
        summary,
        Summary {
            contracts_closed: 2,
            containers_delivered: 6,
            pings_recorded: 24,
            audit_events: summary.audit_events,
            berth_refusals: 1,
            ships_retired: 1,
        },
        "{log}"
    );
    assert!(summary.audit_events > 40, "every fleet action is audited: {}", summary.audit_events);
    // One delivery notice per contract, to its shipper.
    let mut recipients: Vec<Vec<String>> = mailer.delivered_emails().into_iter().map(|email| email.to).collect();
    recipients.sort();
    assert_eq!(recipients, [vec!["grace@helios-freight.example".to_string()], vec!["sally@red-dust-logistics.example".to_string()]]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_week_in_memory() {
    let mailer = MemoryMailer::new();
    let app = lagrange::Lagrange::new(ash_memory::Memory::new(), support::telemetry().await, mailer.clone());
    the_week_runs(app, mailer).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_week_in_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let fleet = ash_sqlite::Sqlite::file(dir.path().join("fleet.db")).await.unwrap();
    fleet.migrate(lagrange::migrations_dir()).await.unwrap();
    let mailer = MemoryMailer::new();
    let app = lagrange::Lagrange::new(fleet, support::telemetry().await, mailer.clone());
    the_week_runs(app, mailer).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_week_in_postgres() {
    let Some(fleet) = support::postgres_fleet().await else {
        return;
    };
    let mailer = MemoryMailer::new();
    let app = lagrange::Lagrange::new(fleet, support::telemetry().await, mailer.clone());
    the_week_runs(app, mailer).await;
}
