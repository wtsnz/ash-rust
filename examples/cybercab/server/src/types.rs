//! Closed sets shared across the contexts, each stored as its lower-case name.

use ash_core::AshEnum;

/// How much a rider has ridden with us.
#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum RiderTier {
    Standard,
    Plus,
    /// Rode in the first week of service.
    Founder,
}

/// What a cab reported that an operator should know about.
#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum AlertKind {
    #[ash(string = "low_battery")]
    LowBattery,
    #[ash(string = "hard_braking")]
    HardBraking,
    /// Something in the road the cab is waiting out.
    Obstruction,
    /// The rider pressed the assist button.
    #[ash(string = "rider_assist")]
    RiderAssist,
    #[ash(string = "sensor_degraded")]
    SensorDegraded,
    #[ash(string = "door_ajar")]
    DoorAjar,
}

/// How urgently an alert needs a human.
#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}
