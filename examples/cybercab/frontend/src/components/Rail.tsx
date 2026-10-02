import { useState } from "react";

import type { Cab, FleetAlert, ServiceZone, Trip } from "../lib/client";
import { client } from "../lib/client";
import { CAB_STATUS, TRIP_STATUS, cabStatus, isActive, journeyOf } from "../lib/model";
import { ago, dollars, duration } from "../lib/geo";
import { JourneyRibbon } from "./JourneyRibbon";

const TIER_MARK: Record<string, string> = { FOUNDER: "Founder", PLUS: "Plus", STANDARD: "" };

/** Trips the feed shows at once. A big fleet has thousands on the road; nobody reads past these. */
const FEED_ROWS = 60;

/** One trip in the feed: route, rider, cab, and its journey ribbon. */
export function TripRow({
  trip,
  cab,
  now,
  selected,
  onSelect,
}: {
  trip: Trip;
  cab?: Cab;
  now: number;
  selected: boolean;
  onSelect: () => void;
}) {
  const journey = journeyOf(trip, cab, now);
  const live = isActive(trip);
  const status = cab ? cabStatus(cab) : undefined;
  return (
    <button className={`trip ${live ? "trip--live" : ""} ${selected ? "is-selected" : ""}`} onClick={onSelect}>
      <div className="trip__top">
        <span className="trip__route">
          {trip.pickup_name}
          <i>→</i>
          {trip.dropoff_name}
        </span>
        <span className={`trip__state trip__state--${trip.status}`}>
          {journey.eta !== undefined ? `${TRIP_STATUS[trip.status]} · ${duration(journey.eta)}` : TRIP_STATUS[trip.status]}
        </span>
      </div>
      <JourneyRibbon journey={journey} />
      <div className="trip__meta">
        <span className="code">{trip.code}</span>
        <span>
          {trip.rider?.display_name}
          {trip.rider && TIER_MARK[trip.rider.tier] && <em className={`tier tier--${trip.rider.tier.toLowerCase()}`}>{TIER_MARK[trip.rider.tier]}</em>}
        </span>
        {cab && (
          <span className="cabchip" style={{ color: status ? CAB_STATUS[status].color : undefined }}>
            {cab.call_sign}
          </span>
        )}
        <span className="trip__fare">
          {dollars(trip.fare_cents)}
          {trip.surge > 1 && <small>{trip.surge.toFixed(1)}×</small>}
        </span>
        <span className="trip__when">{ago(trip.completed_at ?? trip.cancelled_at ?? trip.requested_at, now)}</span>
      </div>
    </button>
  );
}

export function TripFeed({
  trips,
  cabs,
  now,
  selectedCabId,
  onSelectCab,
}: {
  trips: Trip[];
  cabs: Map<string, Cab>;
  now: number;
  selectedCabId?: string;
  onSelectCab: (id?: string) => void;
}) {
  const [filter, setFilter] = useState<"live" | "all">("live");
  const shown = filter === "live" ? trips.filter(isActive) : trips;
  return (
    <section className="feed">
      <div className="segmented" role="tablist">
        <button className={filter === "live" ? "is-on" : ""} onClick={() => setFilter("live")}>
          On the road <b>{trips.filter(isActive).length}</b>
        </button>
        <button className={filter === "all" ? "is-on" : ""} onClick={() => setFilter("all")}>
          Recent
        </button>
      </div>
      <div className="feed__list">
        {shown.length === 0 && <p className="empty">No riders on the road right now.</p>}
        {shown.slice(0, FEED_ROWS).map((trip) => (
          <TripRow
            key={trip.id}
            trip={trip}
            cab={trip.cab_id ? cabs.get(trip.cab_id) : undefined}
            now={now}
            selected={!!trip.cab_id && trip.cab_id === selectedCabId}
            onSelect={() => trip.cab_id && onSelectCab(trip.cab_id)}
          />
        ))}
        {shown.length > FEED_ROWS && (
          <p className="feed__more">
            and {(shown.length - FEED_ROWS).toLocaleString()} more {filter === "live" ? "on the road" : "recent"}
          </p>
        )}
      </div>
    </section>
  );
}

const KIND: Record<string, string> = {
  LOW_BATTERY: "Low battery",
  HARD_BRAKING: "Hard braking",
  OBSTRUCTION: "Obstruction",
  RIDER_ASSIST: "Rider assist",
  SENSOR_DEGRADED: "Sensor degraded",
  DOOR_AJAR: "Door ajar",
};

/** Open alerts, most urgent first, with the controls to handle them. */
export function AlertStack({
  alerts,
  now,
  readOnly,
  onSelectCab,
}: {
  alerts: FleetAlert[];
  now: number;
  readOnly?: boolean;
  onSelectCab: (id?: string) => void;
}) {
  const [busy, setBusy] = useState<string>();
  const rank = { CRITICAL: 0, WARNING: 1, INFO: 2 } as Record<string, number>;
  const sorted = [...alerts].sort(
    (a, b) => rank[a.severity] - rank[b.severity] || Date.parse(b.raised_at) - Date.parse(a.raised_at),
  );
  const act = async (alert: FleetAlert, kind: "acknowledge" | "resolve") => {
    setBusy(alert.id);
    try {
      const at = new Date().toISOString();
      if (kind === "acknowledge") await client.fleetAlert.acknowledge(alert.id, { acknowledged_at: at, handled_by: "Ops desk" });
      else await client.fleetAlert.resolve(alert.id, { resolved_at: at, handled_by: "Ops desk" });
    } finally {
      setBusy(undefined);
    }
  };
  return (
    <section className="alerts">
      {sorted.length === 0 && <p className="empty">All quiet. Nothing needs a person.</p>}
      {sorted.map((alert) => (
        <article key={alert.id} className={`alert alert--${alert.severity.toLowerCase()} alert--${alert.status}`}>
          <div className="alert__head">
            <span className="alert__kind">{KIND[alert.kind] ?? alert.kind}</span>
            <span className="alert__when">{ago(alert.raised_at, now)}</span>
          </div>
          <button className="alert__message" onClick={() => onSelectCab(alert.cab_id)}>
            {alert.message}
          </button>
          {!readOnly && (
            <div className="alert__actions">
              {alert.status === "open" && (
                <button className="ghost" disabled={busy === alert.id} onClick={() => act(alert, "acknowledge")}>
                  Acknowledge
                </button>
              )}
              <button className="ghost" disabled={busy === alert.id} onClick={() => act(alert, "resolve")}>
                Resolve
              </button>
              {alert.status === "acknowledged" && <span className="alert__owner">Ops desk has it</span>}
            </div>
          )}
        </article>
      ))}
    </section>
  );
}

/** Events an operator can stage in a zone, to see the fleet respond. */
const EVENTS: Record<string, string> = {
  DTX: "Sixth Street Saturday",
  SOC: "SoCo First Thursday",
  EAS: "East Side studio tour",
  UTX: "Longhorns home game",
  MUE: "Mueller farmers market",
  DOM: "Q2 match day",
  AUS: "Holiday arrivals rush",
  GIG: "Grand Prix at COTA",
};

/** Demand by zone: riders waiting, the surge, and any event drawing riders there. */
export function ZoneBoard({ zones, readOnly }: { zones: ServiceZone[]; readOnly?: boolean }) {
  const [busy, setBusy] = useState<string>();
  const sorted = [...zones].sort((a, b) => b.surge - a.surge || b.waiting - a.waiting);
  const toggle = async (zone: ServiceZone) => {
    setBusy(zone.id);
    try {
      if (zone.event_name) await client.serviceZone.clearEvent(zone.id, { event_name: null, event_boost: 1 });
      else await client.serviceZone.hostEvent(zone.id, { event_name: EVENTS[zone.code] ?? "Special event", event_boost: 4 });
    } finally {
      setBusy(undefined);
    }
  };
  return (
    <section className="zones">
      {sorted.map((zone) => (
        <div key={zone.id} className={`zone ${zone.event_name ? "zone--event" : ""}`}>
          <div className="zone__name">
            {zone.name}
            {zone.event_name && <span className="zone__event">{zone.event_name}</span>}
          </div>
          <div className="zone__stats">
            <span className={zone.waiting >= 3 ? "is-warn" : ""}>
              <b>{zone.waiting}</b> waiting
            </span>
            <span className={`surge ${zone.surge > 1 ? "surge--on" : ""}`}>{zone.surge.toFixed(1)}×</span>
            {!readOnly && (
              <button className="ghost" disabled={busy === zone.id} onClick={() => toggle(zone)}>
                {zone.event_name ? "End event" : "Stage event"}
              </button>
            )}
          </div>
        </div>
      ))}
    </section>
  );
}
