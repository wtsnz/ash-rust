use ash_core::{CiString, Float, Vector, resource};
use uuid::Uuid;

use super::port::Port;
use crate::types::Atmosphere;

/// Average cruising speed of a freighter between bodies, in AU per hour.
pub const CRUISE_AU_PER_HOUR: f64 = 0.01;

/// The shortest straight-line route between two bodies, from their stored positions.
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub distance_au: f64,
    pub transit_hours: i64,
}

impl Route {
    pub fn between(from: &Vector<3>, to: &Vector<3>) -> Self {
        let distance_au = from
            .as_slice()
            .iter()
            .zip(to.as_slice())
            .map(|(a, b)| f64::from(a - b).powi(2))
            .sum::<f64>()
            .sqrt();
        // Even a hop between a planet and its own orbit takes a few hours of manoeuvring.
        let transit_hours = ((distance_au / CRUISE_AU_PER_HOUR).ceil() as i64).max(6);
        Self {
            distance_au,
            transit_hours,
        }
    }
}

resource! {
    /// A body ships fly between. Positions are heliocentric, in AU, at the epoch the
    /// timetable was planned for.
    Planet {
        table "planets";
        timestamps;

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            name: CiString;
            surface_gravity: Float;
            position: Vector<3>;
            atmosphere: Atmosphere;
        }

        identities {
            identity unique_name: [name], message: "a planet with this name is already charted";
        }

        relationships {
            has_many ports: Port [fk: planet_id];
        }

        aggregates {
            port_count: Option<i64> = count(ports);
        }

        actions {
            create chart {
                primary;
                accept [name, surface_gravity, position, atmosphere];
            }

            read read {
                primary;
            }

            update survey {
                primary;
                accept [surface_gravity, position];
            }

            /// Distance and transit time between two bodies.
            generic route, Route {
                argument from_planet: Uuid;
                argument to_planet: Uuid;

                run |input| async move {
                    let from = Planet::get(input.ctx, input.from_planet).await?;
                    let to = Planet::get(input.ctx, input.to_planet).await?;
                    Ok(Route::between(&from.position, &to.position))
                };
            }
        }

        policies {
            policy action_type(read) | action(route) {
                authorize_if always;
            }
            policy action(chart) | action(survey) {
                authorize_if actor_eq(role = "port_authority");
            }
        }
    }
}
