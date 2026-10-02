//! High-volume data that lives in its own database: transponder pings and the audit
//! log of every fleet action. Resources here are tagged `store TelemetryStore;`, so a
//! [`ash_core::StoreRegistry`] routes them away from the fleet database.

pub mod ops_event;
pub mod ping;

pub use ops_event::{AuditNotifier, OpsEvent};
pub use ping::TelemetryPing;

/// The telemetry database.
pub struct TelemetryStore;
impl ash_core::StoreTag for TelemetryStore {}
