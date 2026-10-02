//! A: the command center's reads, at rising concurrency, with the city standing still.

use std::sync::Arc;

use serde_json::{Value, json};

use super::{Plan, load};
use std::time::Duration;

use crate::gql::{Graphql, Response};
use crate::record::Record;
use crate::server::Running;

pub const CAB_QUERY: &str = "query Cab($id: ID!) {
  getCab(id: $id) { id callSign status lng lat headingDeg speedKph batteryPct depot { name } }
}";

const FLEET_PAGE: &str = "query Fleet($after: String) {
  listCabs(first: 250, after: $after) {
    results { id callSign status lng lat headingDeg speedKph batteryPct } endKeyset
  }
}";

/// The whole fleet, as the SDK's `all()` reads it: page after page of the most a page holds
/// (250), each after the last's end keyset, as both servers page every read.
pub async fn whole_fleet(api: &Graphql) -> Response {
    let mut elapsed = Duration::ZERO;
    let mut after = Value::Null;
    loop {
        let page = api.request(FLEET_PAGE, json!({ "after": after })).await;
        elapsed += page.elapsed;
        let list = &page.data["listCabs"];
        let read = list["results"].as_array().map_or(0, Vec::len);
        if page.failed || read < 250 || list["endKeyset"].is_null() {
            return Response { data: Value::Null, elapsed, failed: page.failed };
        }
        after = list["endKeyset"].clone();
    }
}

pub const TRIPS_QUERY: &str = "{
  listTrips(sort: [{ field: REQUESTED_AT, order: DESC }], first: 50) {
    results { code status fareCents rider { displayName tier } cab { callSign } zone { name } }
  }
}";

pub const AGGREGATES_QUERY: &str = "{
  listCabs(sort: [{ field: CALL_SIGN }], first: 100) { results { callSign tripsCompleted faresCents } }
}";

const KEYSET_PAGE: &str = "query Page($after: String) {
  listTrips(sort: [{ field: REQUESTED_AT, order: DESC }], first: 25, after: $after) {
    results { id code status } endKeyset
  }
}";

/// A page as the SDK's `page()` reads it: with the count of every matching record.
const COUNTED_PAGE: &str = "{
  listTrips(sort: [{ field: REQUESTED_AT, order: DESC }], first: 25) {
    count results { id code status } endKeyset
  }
}";

/// The reads, by name, as the report shows them.
pub const QUERIES: [&str; 6] =
    ["cab_by_id", "fleet", "trips_with_riders", "aggregates", "keyset_page", "counted_page"];

pub async fn run(plan: &Plan, server: &Running, fleet: usize, rep: usize) -> Vec<Record> {
    let api = server.api.clone();
    let ids: Vec<String> = api
        .must("{ listCabs { results { id } } }", json!({}))
        .await["listCabs"]["results"]
        .as_array()
        .expect("cabs")
        .iter()
        .map(|cab| cab["id"].as_str().expect("an id").to_string())
        .collect();
    let ids = Arc::new(ids);
    let after = api.must(KEYSET_PAGE, json!({ "after": Value::Null })).await["listTrips"]["endKeyset"].clone();

    let mut records = Vec::new();
    for query in QUERIES.into_iter().filter(|q| plan.queries.is_empty() || plan.queries.iter().any(|w| w == q)) {
        for &workers in &plan.levels {
            let api: Graphql = api.clone();
            let ids = Arc::clone(&ids);
            let after = after.clone();
            let result = load::run(workers, plan.warmup, plan.measure, move |worker, n| {
                let api = api.clone();
                let ids = Arc::clone(&ids);
                let after = after.clone();
                async move {
                    match query {
                        "cab_by_id" => {
                            let id = &ids[(worker * 7919 + n as usize) % ids.len()];
                            api.request(CAB_QUERY, json!({ "id": id })).await
                        }
                        "fleet" => whole_fleet(&api).await,
                        "trips_with_riders" => api.request(TRIPS_QUERY, json!({})).await,
                        "aggregates" => api.request(AGGREGATES_QUERY, json!({})).await,
                        "counted_page" => api.request(COUNTED_PAGE, json!({})).await,
                        _ => api.request(KEYSET_PAGE, json!({ "after": after })).await,
                    }
                }
            })
            .await;
            records.push(Record {
                scenario: "reads".into(),
                server: server.label.clone(),
                params: json!({ "query": query, "concurrency": workers, "fleet": fleet }),
                metrics: serde_json::to_value(result).expect("json"),
                rep,
            });
        }
    }
    records
}
