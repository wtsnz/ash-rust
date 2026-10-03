//! Austin as the simulation sees it: the places riders go, the Supercharger hubs, the
//! service zones, and real driving routes between every pair of stops, baked from
//! OpenStreetMap by `tools/bake-routes.mjs`.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;

const AUSTIN: &str = include_str!("../../../shared/cybercab/austin.json");
const ROUTES: &str = include_str!("../../../shared/cybercab/routes.json");

/// `[lng, lat]`, as GeoJSON orders them.
pub type Point = [f64; 2];

#[derive(Clone, Debug, Deserialize)]
pub struct ZoneSpec {
    pub code: String,
    pub name: String,
    pub center: Point,
    pub radius_m: i64,
    pub base_demand: f64,
}

/// Somewhere a cab can stop: a place riders go, or a hub.
#[derive(Clone, Debug, Deserialize)]
pub struct Stop {
    pub code: String,
    pub name: String,
    pub zone: String,
    pub at: Point,
    /// Charging stalls, for a hub.
    #[serde(default)]
    pub stalls: Option<i64>,
}

#[derive(Deserialize)]
struct CityFile {
    center: Point,
    zones: Vec<ZoneSpec>,
    places: Vec<Stop>,
    depots: Vec<Stop>,
}

#[derive(Deserialize)]
struct BakedRoute {
    polyline: String,
    distance_m: f64,
    duration_s: f64,
}

#[derive(Deserialize)]
struct RouteFile {
    routes: HashMap<String, BakedRoute>,
}

/// A drivable path between two stops, with the distance along it at every vertex.
#[derive(Clone, Debug)]
pub struct Route {
    pub points: Vec<Point>,
    cumulative: Vec<f64>,
    pub distance_m: f64,
    pub duration_s: f64,
}

impl Route {
    fn new(points: Vec<Point>, duration_s: f64) -> Self {
        let mut cumulative = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (i, point) in points.iter().enumerate() {
            if i > 0 {
                total += metres_between(points[i - 1], *point);
            }
            cumulative.push(total);
        }
        Self {
            points,
            cumulative,
            distance_m: total,
            duration_s,
        }
    }

    /// Staying put, for a cab already where it's going.
    pub fn stationary(at: Point) -> Self {
        Self::new(vec![at, at], 0.0)
    }

    fn reversed(&self) -> Self {
        let mut points = self.points.clone();
        points.reverse();
        Self::new(points, self.duration_s)
    }

    /// Average driving speed along the route, in metres a second.
    pub fn speed_mps(&self) -> f64 {
        if self.duration_s > 0.0 {
            (self.distance_m / self.duration_s).clamp(4.0, 30.0)
        } else {
            10.0
        }
    }

    /// Where a cab `travelled` metres along the route is, and which way it faces.
    pub fn position(&self, travelled: f64) -> (Point, f64) {
        let travelled = travelled.clamp(0.0, self.distance_m);
        let segment = self
            .cumulative
            .windows(2)
            .position(|pair| travelled <= pair[1])
            .unwrap_or(self.points.len().saturating_sub(2));
        let (a, b) = (
            self.points[segment],
            self.points[(segment + 1).min(self.points.len() - 1)],
        );
        let length =
            self.cumulative.get(segment + 1).copied().unwrap_or(0.0) - self.cumulative[segment];
        let t = if length > 0.0 {
            (travelled - self.cumulative[segment]) / length
        } else {
            1.0
        };
        let at = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        (at, bearing(a, b))
    }

    pub fn encode(&self) -> String {
        encode_polyline(&self.points)
    }
}

/// The city: its zones, stops and the routes between them.
pub struct City {
    pub center: Point,
    pub zones: Vec<ZoneSpec>,
    pub places: Vec<Stop>,
    pub depots: Vec<Stop>,
    routes: HashMap<(String, String), Arc<Route>>,
}

impl City {
    pub fn austin() -> Arc<Self> {
        let file: CityFile = serde_json::from_str(AUSTIN).expect("austin.json");
        let baked: RouteFile = serde_json::from_str(ROUTES).expect("routes.json");
        let mut routes = HashMap::new();
        for (key, route) in baked.routes {
            let (from, to) = key.split_once('|').expect("route keys are FROM|TO");
            let forward = Route::new(decode_polyline(&route.polyline), route.duration_s);
            debug_assert!(
                (forward.distance_m - route.distance_m).abs() < route.distance_m * 0.2 + 50.0
            );
            routes.insert(
                (to.to_string(), from.to_string()),
                Arc::new(forward.reversed()),
            );
            routes.insert((from.to_string(), to.to_string()), Arc::new(forward));
        }
        Arc::new(Self {
            center: file.center,
            zones: file.zones,
            places: file.places,
            depots: file.depots,
            routes,
        })
    }

    pub fn stop(&self, code: &str) -> &Stop {
        self.places
            .iter()
            .chain(&self.depots)
            .find(|stop| stop.code == code)
            .unwrap_or_else(|| panic!("no stop {code}"))
    }

    /// The driving route between two stops.
    pub fn route(&self, from: &str, to: &str) -> Arc<Route> {
        if from == to {
            return Arc::new(Route::stationary(self.stop(from).at));
        }
        self.routes
            .get(&(from.to_string(), to.to_string()))
            .cloned()
            .unwrap_or_else(|| panic!("no route {from} → {to}"))
    }

    pub fn places_in(&self, zone: &str) -> impl Iterator<Item = &Stop> {
        self.places.iter().filter(move |place| place.zone == zone)
    }

    /// The hub nearest `at`.
    pub fn nearest_depot(&self, at: Point) -> &Stop {
        self.depots
            .iter()
            .min_by(|a, b| metres_between(a.at, at).total_cmp(&metres_between(b.at, at)))
            .expect("the city has hubs")
    }
}

/// Great-circle distance in metres.
pub fn metres_between(a: Point, b: Point) -> f64 {
    let (lat1, lat2) = (a[1].to_radians(), b[1].to_radians());
    let dlat = lat2 - lat1;
    let dlng = (b[0] - a[0]).to_radians();
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlng / 2.0).sin().powi(2);
    6_371_000.0 * 2.0 * h.sqrt().asin()
}

/// Compass bearing from `a` to `b`, in degrees.
pub fn bearing(a: Point, b: Point) -> f64 {
    let (lat1, lat2) = (a[1].to_radians(), b[1].to_radians());
    let dlng = (b[0] - a[0]).to_radians();
    let y = dlng.sin() * lat2.cos();
    let x = lat1.cos() * lat2.sin() - lat1.sin() * lat2.cos() * dlng.cos();
    (y.atan2(x).to_degrees() + 360.0) % 360.0
}

/// Decodes a precision-5 encoded polyline into `[lng, lat]` points.
pub fn decode_polyline(encoded: &str) -> Vec<Point> {
    let bytes = encoded.as_bytes();
    let (mut index, mut lat, mut lng) = (0, 0i64, 0i64);
    let mut points = Vec::new();
    while index < bytes.len() {
        let mut next = || {
            let (mut shift, mut result) = (0, 0i64);
            loop {
                let byte = i64::from(bytes[index]) - 63;
                index += 1;
                result |= (byte & 0x1f) << shift;
                shift += 5;
                if byte < 0x20 {
                    break;
                }
            }
            if result & 1 == 1 {
                !(result >> 1)
            } else {
                result >> 1
            }
        };
        lat += next();
        lng += next();
        points.push([lng as f64 / 1e5, lat as f64 / 1e5]);
    }
    points
}

/// Encodes `[lng, lat]` points as a precision-5 polyline.
pub fn encode_polyline(points: &[Point]) -> String {
    let mut out = String::new();
    let (mut last_lat, mut last_lng) = (0i64, 0i64);
    let mut push = |value: i64| {
        let mut v = if value < 0 { !(value << 1) } else { value << 1 };
        while v >= 0x20 {
            out.push(char::from((0x20 | (v & 0x1f)) as u8 + 63));
            v >>= 5;
        }
        out.push(char::from(v as u8 + 63));
    };
    for point in points {
        let (lat, lng) = (
            (point[1] * 1e5).round() as i64,
            (point[0] * 1e5).round() as i64,
        );
        push(lat - last_lat);
        push(lng - last_lng);
        last_lat = lat;
        last_lng = lng;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polylines_round_trip() {
        let points = vec![
            [-97.74285, 30.2682],
            [-97.7404, 30.2747],
            [-97.6668, 30.2024],
        ];
        assert_eq!(decode_polyline(&encode_polyline(&points)), points);
    }

    #[test]
    fn every_pair_of_stops_has_a_route_both_ways() {
        let city = City::austin();
        let stops: Vec<&Stop> = city.places.iter().chain(&city.depots).collect();
        for a in &stops {
            for b in &stops {
                let route = city.route(&a.code, &b.code);
                if a.code != b.code {
                    assert!(route.distance_m > 100.0, "{} → {}", a.code, b.code);
                    // Routes start and end near the stops they join.
                    assert!(
                        metres_between(route.points[0], a.at) < 300.0,
                        "{} → {}",
                        a.code,
                        b.code
                    );
                }
            }
        }
    }

    #[test]
    fn positions_follow_the_route() {
        let city = City::austin();
        let route = city.route("CON6", "AUSA");
        let (start, _) = route.position(0.0);
        let (end, _) = route.position(route.distance_m);
        assert_eq!(start, route.points[0]);
        assert_eq!(end, *route.points.last().unwrap());
        let (mid, heading) = route.position(route.distance_m / 2.0);
        assert!(metres_between(mid, start) > 1000.0);
        assert!((0.0..360.0).contains(&heading));
    }
}
