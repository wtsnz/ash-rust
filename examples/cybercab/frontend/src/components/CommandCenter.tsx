import { useEffect, useMemo, useRef, useState } from "react";

import {
  client,
  type Cab,
  type Trip,
  useAshConnectionStatus,
  useCabLive,
  useDepotLive,
  useFleetAlertLive,
  usePulseSampleLive,
  useServiceZoneLive,
  useTelemetrySampleLive,
  useTripLive,
} from "../lib/client";
import { ACTIVE_TRIP_STATES, OPEN_ALERT_STATES, CAB_STATUS, cabStatus, journeyOf } from "../lib/model";
import { duration } from "../lib/geo";
import { CabPanel } from "./CabPanel";
import { FleetMap } from "./FleetMap";
import { JourneyRibbon } from "./JourneyRibbon";
import { PulseStrip } from "./PulseStrip";
import { AlertStack, TripFeed, TripRow, ZoneBoard } from "./Rail";

/** Seconds the wall's spotlight stays on one ride. */
const SPOTLIGHT_S = 14;

function useNow(intervalMs = 1000) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), intervalMs);
    return () => clearInterval(timer);
  }, [intervalMs]);
  return now;
}

/**
 * The command center: the city with the whole fleet on it, the rides on the road, what
 * needs a person, and demand across town. Everything on screen is a live query, kept in
 * sync with the server by the generated SDK.
 *
 * Console mode is for the desk: dense, with every command to hand. Wall mode is for the
 * screen across the room: bigger, read-only, and it follows one ride at a time.
 */
export function CommandCenter({ initialMode = "console" }: { initialMode?: "console" | "wall" }) {
  const [wall, setWall] = useState(initialMode === "wall");
  const [tab, setTab] = useState<"trips" | "alerts" | "zones">("trips");
  const [selectedCabId, setSelectedCabId] = useState<string>();
  const now = useNow();
  const status = useAshConnectionStatus(client);

  // The live state of the room.
  const { data: cabs } = useCabLive(client);
  const { data: active } = useTripLive(
    client,
    { filter: { status: { in: [...ACTIVE_TRIP_STATES] } }, sort: [{ field: "requested_at", order: "desc" }], include: { rider: true } },
    { syncDelayMs: 300 },
  );
  const { data: recent } = useTripLive(
    client,
    { sort: [{ field: "requested_at", order: "desc" }], limit: 50, include: { rider: true } },
    { syncDelayMs: 800 },
  );
  const { data: alerts } = useFleetAlertLive(
    client,
    { filter: { status: { in: [...OPEN_ALERT_STATES] } }, sort: [{ field: "raised_at", order: "desc" }] },
    { syncDelayMs: 300 },
  );
  const { data: pulse } = usePulseSampleLive(client, { sort: [{ field: "recorded_at", order: "desc" }], limit: 72 }, { syncDelayMs: 300 });
  const { data: zones } = useServiceZoneLive(client);
  const { data: depots } = useDepotLive(client);
  const { data: trail } = useTelemetrySampleLive(
    client,
    { filter: { cab_id: { eq: selectedCabId ?? "" } }, sort: [{ field: "recorded_at", order: "desc" }], limit: 120 },
    { enabled: !!selectedCabId, syncDelayMs: 1500 },
  );

  const cabsById = useMemo(() => new Map(cabs.map((cab) => [cab.id, cab])), [cabs]);
  const tripsById = useMemo(() => new Map(active.map((trip) => [trip.id, trip])), [active]);
  const selected = selectedCabId ? cabsById.get(selectedCabId) : undefined;
  const selectedTrip = selected?.trip_id ? tripsById.get(selected.trip_id) : undefined;
  const critical = alerts.filter((a) => a.severity === "CRITICAL" && a.status === "open").length;

  // Wall mode: the spotlight stays on a ride for a while, then moves to the next one,
  // and moves on early when its ride ends.
  const fleet = useRef(cabs);
  fleet.current = cabs;
  useEffect(() => {
    if (!wall) return;
    let since = 0;
    let current: string | undefined;
    const check = () => {
      const riding = fleet.current.filter((cab) => cab.status === "on_trip");
      const stillRiding = riding.some((cab) => cab.id === current);
      if (riding.length === 0 || (stillRiding && Date.now() - since < SPOTLIGHT_S * 1000)) return;
      const index = riding.findIndex((cab) => cab.id === current);
      current = riding[(index + 1) % riding.length].id;
      since = Date.now();
      setSelectedCabId(current);
    };
    check();
    const timer = setInterval(check, 1000);
    return () => clearInterval(timer);
  }, [wall]);

  useEffect(() => {
    document.documentElement.style.setProperty("--scale", wall ? "1.32" : "1");
    const url = new URL(window.location.href);
    if (wall) url.searchParams.set("mode", "wall");
    else url.searchParams.delete("mode");
    window.history.replaceState(null, "", url);
  }, [wall]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.target instanceof HTMLInputElement) return;
      if (event.key === "w" || event.key === "W") setWall((w) => !w);
      if (event.key === "Escape") setSelectedCabId(undefined);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className={`room ${wall ? "room--wall" : "room--console"}`}>
      <PulseStrip cabs={cabs} pulse={pulse} status={status} now={now} wall={wall} onToggleWall={() => setWall((w) => !w)} />

      <main className="stage">
        <FleetMap
          cabs={cabs}
          trips={tripsById}
          zones={zones}
          depots={depots}
          alerts={alerts}
          trail={selectedCabId ? trail : []}
          selectedCabId={selectedCabId}
          follow={wall}
          wall={wall}
          onSelectCab={setSelectedCabId}
        />

        <MapLegend cabs={cabs} />

        {wall ? (
          <>
            <aside className="wallrail">
              <h2>
                On the road <b>{active.length}</b>
                {critical > 0 && <span className="wallrail__alarm">{critical} need a person</span>}
              </h2>
              <div className="wallrail__list">
                {active
                  .filter((trip) => trip.status !== "requested")
                  .slice(0, 9)
                  .map((trip) => (
                    <TripRow
                      key={trip.id}
                      trip={trip}
                      cab={trip.cab_id ? cabsById.get(trip.cab_id) : undefined}
                      now={now}
                      selected={trip.cab_id === selectedCabId}
                      onSelect={() => trip.cab_id && setSelectedCabId(trip.cab_id)}
                    />
                  ))}
              </div>
            </aside>
            {selected && <Spotlight cab={selected} trip={selectedTrip} now={now} />}
            <Ticker trips={recent} now={now} />
          </>
        ) : (
          <>
            {selected && (
              <CabPanel
                cab={selected}
                trip={selectedTrip}
                depot={depots.find((d) => d.id === selected.depot_id)}
                now={now}
                onClose={() => setSelectedCabId(undefined)}
              />
            )}
            <aside className="rail">
              <nav className="tabs" role="tablist">
                <button className={tab === "trips" ? "is-on" : ""} onClick={() => setTab("trips")}>
                  Trips
                </button>
                <button className={tab === "alerts" ? "is-on" : ""} onClick={() => setTab("alerts")}>
                  Alerts
                  {alerts.filter((a) => a.status === "open").length > 0 && (
                    <b className={critical ? "is-critical" : ""}>{alerts.filter((a) => a.status === "open").length}</b>
                  )}
                </button>
                <button className={tab === "zones" ? "is-on" : ""} onClick={() => setTab("zones")}>
                  Zones
                </button>
              </nav>
              {tab === "trips" && (
                <TripFeed
                  trips={tab === "trips" ? mergeTrips(active, recent) : []}
                  cabs={cabsById}
                  now={now}
                  selectedCabId={selectedCabId}
                  onSelectCab={setSelectedCabId}
                />
              )}
              {tab === "alerts" && <AlertStack alerts={alerts} now={now} onSelectCab={setSelectedCabId} />}
              {tab === "zones" && <ZoneBoard zones={zones} />}
            </aside>
          </>
        )}
      </main>
    </div>
  );
}

/** Active trips first, then the rest of the recent ones, without repeats. */
function mergeTrips(active: Trip[], recent: Trip[]): Trip[] {
  const seen = new Set(active.map((t) => t.id));
  return [...active, ...recent.filter((t) => !seen.has(t.id))];
}

function MapLegend({ cabs }: { cabs: Cab[] }) {
  const moving = cabs.filter((cab) => cab.speed_kph > 0).length;
  return (
    <div className="legend">
      {(["on_trip", "dispatched", "available", "charging"] as const).map((status) => (
        <span key={status}>
          <i style={{ background: CAB_STATUS[status].color }} />
          {CAB_STATUS[status].label}
        </span>
      ))}
      <span className="legend__moving">{moving} moving</span>
    </div>
  );
}

/** Wall mode's ride in focus: who's aboard, where to, and how far along. */
function Spotlight({ cab, trip, now }: { cab: Cab; trip?: Trip; now: number }) {
  const journey = trip ? journeyOf(trip, cab, now) : undefined;
  const status = cabStatus(cab);
  return (
    <section className="spotlight" key={cab.id}>
      <div className="spotlight__kicker">
        <span style={{ color: CAB_STATUS[status].color }}>●</span> {cab.call_sign} · “{cab.nickname}” · {cab.speed_kph} km/h · {cab.battery_pct}%
      </div>
      {trip && journey ? (
        <>
          <div className="spotlight__route">
            {trip.pickup_name} <i>→</i> {trip.dropoff_name}
          </div>
          <div className="spotlight__who">
            {trip.rider?.display_name}
            {journey.eta !== undefined && <span> · arriving in {duration(journey.eta)}</span>}
          </div>
          <JourneyRibbon journey={journey} size="wall" />
        </>
      ) : (
        <div className="spotlight__route">{CAB_STATUS[status].label}</div>
      )}
    </section>
  );
}

/** Wall mode's running log of rides finishing and starting. */
function Ticker({ trips, now }: { trips: Trip[]; now: number }) {
  const events = trips.slice(0, 14).map((trip) => {
    const verb =
      trip.status === "completed"
        ? `dropped off at ${trip.dropoff_name}`
        : trip.status === "cancelled"
          ? "cancelled"
          : trip.status === "requested"
            ? `hailing from ${trip.pickup_name}`
            : `${trip.pickup_name} → ${trip.dropoff_name}`;
    const when = Math.round((now - Date.parse(trip.completed_at ?? trip.requested_at)) / 60_000);
    return `${trip.rider?.display_name ?? "Rider"} ${verb} · ${when <= 0 ? "now" : `${when}m`}`;
  });
  return (
    <div className="ticker" aria-hidden>
      <div className="ticker__track">
        {[...events, ...events].map((text, i) => (
          <span key={i}>{text}</span>
        ))}
      </div>
    </div>
  );
}
