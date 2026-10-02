//! B: operator commands at rising concurrency, while the fleet moves: pulling a cab over
//! and sending it on again, and a trip's whole life through its state machine. Each
//! client works its own cabs, so no two contend; neither command fights the simulation.

use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::{Plan, load};
use crate::record::Record;
use crate::server::Running;

/// A client's trip, and the step of its life it's at.
type TripSlot = Mutex<(Option<String>, u8)>;

/// A cab pulled over at the curb and sent on again, whatever it's doing.
const CAB_COMMANDS: [&str; 2] = ["pullOverCab", "resumeCab"];

pub async fn run(plan: &Plan, server: &Running, fleet: usize, rep: usize) -> Vec<Record> {
    let api = server.api.clone();
    let cabs: Vec<String> = ids(
        &api.must("{ listCabs { results { id } } }", json!({})).await["listCabs"],
    );
    let riders = ids(&api.must("{ listRiders { results { id } } }", json!({})).await["listRiders"]);
    let zones = ids(&api.must("{ listServiceZones { results { id } } }", json!({})).await["listServiceZones"]);
    let (cabs, riders, zones) = (Arc::new(cabs), Arc::new(riders), Arc::new(zones));

    let mut records = Vec::new();
    for &workers in &plan.levels {
        assert!(cabs.len() >= workers, "a cab for every client");

        // Each command a request; each client loops its own cabs through them.
        let commands = {
            let (api, cabs) = (api.clone(), Arc::clone(&cabs));
            load::run(workers, plan.warmup, plan.measure, move |worker, n| {
                let (api, cabs) = (api.clone(), Arc::clone(&cabs));
                async move {
                    let mine = (cabs.len() - worker - 1) / workers + 1;
                    let cab = &cabs[worker + workers * ((n as usize / CAB_COMMANDS.len()) % mine)];
                    let command = CAB_COMMANDS[n as usize % CAB_COMMANDS.len()];
                    let mutation =
                        format!("mutation($id: ID!) {{ {command}(id: $id) {{ result {{ status halted }} errors {{ message }} }} }}");
                    api.request(&mutation, json!({ "id": cab })).await
                }
            })
            .await
        };
        records.push(Record {
            scenario: "commands".into(),
            server: server.label.clone(),
            params: json!({ "kind": "cab_commands", "concurrency": workers, "fleet": fleet }),
            metrics: serde_json::to_value(commands).expect("json"),
            rep,
        });

        // A trip's whole life, a step a request: requested, assigned, arrived, boarded
        // and completed.
        // Each client's trip and the step it's at; a failed step starts a new trip.
        let trips: Arc<Vec<TripSlot>> = Arc::new((0..workers).map(|_| Mutex::new((None, 0))).collect());
        let label = format!("{}-{rep}-{workers}", server.label);
        let lifecycle = {
            let (api, cabs, riders, zones) = (api.clone(), Arc::clone(&cabs), Arc::clone(&riders), Arc::clone(&zones));
            load::run(workers, plan.warmup, plan.measure, move |worker, n| {
                let (api, cabs, riders, zones, trips) =
                    (api.clone(), Arc::clone(&cabs), Arc::clone(&riders), Arc::clone(&zones), Arc::clone(&trips));
                let label = label.clone();
                async move {
                    let now = chrono::Utc::now().to_rfc3339();
                    let mut slot = trips[worker].lock().await;
                    let (trip, step) = &mut *slot;
                    let (mutation, variables) = match (*step, trip.as_ref()) {
                        (_, None) => (
                            "mutation($input: RequestTripInput!) { requestTrip(input: $input) { result { id } errors { message } } }",
                            json!({ "input": {
                                "code": format!("B-{label}-{worker}-{n}"),
                                "riderId": riders[(worker + n as usize) % riders.len()],
                                "zoneId": zones[(worker + n as usize) % zones.len()],
                                "pickupName": "Congress & 6th", "pickupLng": -97.74285, "pickupLat": 30.2682,
                                "dropoffName": "Texas State Capitol", "dropoffLng": -97.74045, "dropoffLat": 30.2747,
                                "ridePolyline": "cwvwDhkqsQyIaCgCbNoj@eOfC_NxBlAbD|@t@jA",
                                "distanceM": 1724, "durationS": 224, "surge": 1.0, "fareCents": 600,
                                "requestedAt": now,
                            }}),
                        ),
                        (1, Some(id)) => (
                            "mutation($id: ID!, $input: AssignTripInput) { assignTrip(id: $id, input: $input) { result { status } errors { message } } }",
                            json!({ "id": id, "input": { "cabId": cabs[worker], "assignedAt": now, "pickupEtaAt": now } }),
                        ),
                        (2, Some(id)) => (
                            "mutation($id: ID!, $input: ArriveTripInput) { arriveTrip(id: $id, input: $input) { result { status } errors { message } } }",
                            json!({ "id": id, "input": { "arrivedAt": now } }),
                        ),
                        (3, Some(id)) => (
                            "mutation($id: ID!, $input: BoardTripInput) { boardTrip(id: $id, input: $input) { result { status } errors { message } } }",
                            json!({ "id": id, "input": { "pickedUpAt": now, "dropoffEtaAt": now } }),
                        ),
                        (_, Some(id)) => (
                            "mutation($id: ID!, $input: CompleteTripInput) { completeTrip(id: $id, input: $input) { result { status } errors { message } } }",
                            json!({ "id": id, "input": { "completedAt": now, "rating": 5 } }),
                        ),
                    };
                    let response = api.request(mutation, variables).await;
                    if response.failed {
                        *trip = None;
                    } else if trip.is_none() {
                        *trip = response.data["requestTrip"]["result"]["id"].as_str().map(str::to_string);
                        *step = 1;
                    } else if *step == 4 {
                        *trip = None;
                    } else {
                        *step += 1;
                    }
                    response
                }
            })
            .await
        };
        records.push(Record {
            scenario: "commands".into(),
            server: server.label.clone(),
            params: json!({ "kind": "trip_lifecycle", "concurrency": workers, "fleet": fleet }),
            metrics: serde_json::to_value(lifecycle).expect("json"),
            rep,
        });
    }
    records
}

fn ids(list: &Value) -> Vec<String> {
    list["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|record| record["id"].as_str().expect("an id").to_string())
        .collect()
}
