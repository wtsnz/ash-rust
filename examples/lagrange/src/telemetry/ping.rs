use ash_core::{ChangeContext, Error, Float, Inet, Result, UtcDateTime, Vector, resource};
use uuid::Uuid;

use super::TelemetryStore;

/// A transponder only reports for the ship it is fitted to.
pub fn stamp_ship(ctx: &mut ChangeContext<'_>) -> Result<()> {
    let ship_id = ctx
        .actor
        .and_then(|actor| actor.attr("ship_id"))
        .cloned()
        .ok_or(Error::Forbidden)?;
    ctx.fields.insert("ship_id".into(), ship_id);
    Ok(())
}

resource! {
    /// One position report from a ship's transponder.
    TelemetryPing {
        table "telemetry_pings";
        store TelemetryStore;

        actor {
            kind: String;
        }

        attributes {
            id: Uuid [pk];
            transponder_id: Uuid;
            ship_id: Uuid;
            position: Vector<3>;
            speed_kms: Float;
            fuel_pct: i64;
            recorded_at: UtcDateTime;
            source_ip: Inet;
        }

        checks {
            check fuel_range: "fuel_pct BETWEEN 0 AND 100";
        }

        indexes {
            index by_ship_time: [ship_id, recorded_at];
            index by_time: [recorded_at], using: brin;
        }

        actions {
            create report {
                primary;
                accept [position, speed_kms, fuel_pct, recorded_at, source_ip];
                change relate_actor(transponder_id);
                change func(stamp_ship);
            }

            read read {
                primary;
            }
        }

        policies {
            policy action(report) {
                authorize_if actor_eq(kind = "transponder");
            }
            policy action_type(read) {
                authorize_if actor_present;
            }
        }
    }
}
