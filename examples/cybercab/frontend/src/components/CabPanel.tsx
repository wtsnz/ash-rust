import { useState } from "react";

import type { Cab, Depot, Trip } from "../lib/client";
import { client } from "../lib/client";
import { CAB_STATUS, TRIP_STATUS, cabStatus, journeyOf } from "../lib/model";
import { clock, dollars, duration, km } from "../lib/geo";
import { JourneyRibbon } from "./JourneyRibbon";

/** A cab's battery as a bar that turns amber, then red, as it runs down. */
function Battery({ pct, range }: { pct: number; range: number }) {
  const tone = pct < 20 ? "var(--brake)" : pct < 35 ? "var(--sodium)" : "var(--charge)";
  return (
    <div className="gauge">
      <div className="gauge__label">Battery</div>
      <div className="gauge__value">
        {pct}
        <small>%</small>
      </div>
      <div className="gauge__bar">
        <div style={{ width: `${pct}%`, background: tone }} />
      </div>
      <div className="gauge__sub">{range} km range</div>
    </div>
  );
}

function initials(name?: string) {
  return (name ?? "?")
    .split(/\s+/)
    .map((part) => part[0])
    .join("")
    .slice(0, 2);
}

/** Everything about one cab, and what an operator can tell it to do. */
export function CabPanel({
  cab,
  trip,
  depot,
  now,
  readOnly,
  onClose,
}: {
  cab: Cab;
  trip?: Trip;
  depot?: Depot;
  now: number;
  readOnly?: boolean;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState<string>();
  const [error, setError] = useState<string>();
  const status = cabStatus(cab);
  const journey = trip ? journeyOf(trip, cab, now) : undefined;

  const command = async (name: string, run: () => Promise<unknown>) => {
    setBusy(name);
    setError(undefined);
    try {
      await run();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(undefined);
    }
  };

  const milestones: Array<[string, string | null | undefined]> = trip
    ? [
        ["Hailed", trip.requestedAt],
        ["Cab assigned", trip.assignedAt],
        ["At the curb", trip.arrivedAt],
        ["Picked up", trip.pickedUpAt],
        ["Dropped off", trip.completedAt],
      ]
    : [];

  return (
    <aside className="cabpanel" aria-label={`${cab.callSign} details`}>
      <header className="cabpanel__head">
        <div>
          <div className="cabpanel__sign">{cab.callSign}</div>
          <div className="cabpanel__nick">
            “{cab.nickname}” · {cab.software}
          </div>
        </div>
        <button className="ghost icon" onClick={onClose} aria-label="Close">
          ✕
        </button>
      </header>

      <div className="cabpanel__status" style={{ color: CAB_STATUS[status].color }}>
        <i style={{ background: CAB_STATUS[status].color }} />
        {CAB_STATUS[status].label}
        {cab.halted && <span className="halted">Pulled over</span>}
      </div>

      <div className="cabpanel__telemetry">
        <Battery pct={cab.batteryPct} range={cab.rangeKm} />
        <div className="readout">
          <div className="readout__label">Speed</div>
          <div className="readout__value">
            {cab.speedKph}
            <small>km/h</small>
          </div>
          <div className="readout__sub">heading {cab.headingDeg}°</div>
        </div>
        <div className="readout">
          <div className="readout__label">Cabin</div>
          <div className="readout__value">
            {cab.cabinTempC.toFixed(1)}
            <small>°C</small>
          </div>
          <div className="readout__sub">{Math.round(cab.odometerKm).toLocaleString()} km total</div>
        </div>
      </div>

      {trip && journey ? (
        <section className="ride">
          <div className="ride__head">
            <span className="ride__title">{TRIP_STATUS[trip.status]}</span>
            {journey.eta !== undefined && (
              <span className="ride__eta">
                {trip.status === "assigned" ? "pickup in" : "drop-off in"} <b>{duration(journey.eta)}</b>
              </span>
            )}
          </div>

          <div className="passenger">
            <div className={`passenger__avatar tier--${(trip.rider?.tier ?? "STANDARD").toLowerCase()}`}>{initials(trip.rider?.displayName)}</div>
            <div className="passenger__who">
              <div className="passenger__name">{trip.rider?.displayName ?? "Rider"}</div>
              <div className="passenger__sub">
                ★ {trip.rider?.rating.toFixed(2)} · {trip.rider?.tier.toLowerCase()} · ••{trip.rider?.phoneLast4}
                {trip.rider?.assistedBoarding && <span className="flag">Assisted boarding</span>}
              </div>
            </div>
            <div className="passenger__fare">
              {dollars(trip.fareCents)}
              {trip.surge > 1 && <small>{trip.surge.toFixed(1)}× surge</small>}
            </div>
          </div>

          <div className="leg">
            <div className="leg__stop">
              <i className="dot dot--approach" />
              {trip.pickupName}
            </div>
            <div className="leg__stop">
              <i className="dot dot--ride" />
              {trip.dropoffName}
            </div>
            <div className="leg__dist">
              {km(trip.distanceM)} · {trip.code}
            </div>
          </div>

          <JourneyRibbon journey={journey} size="detail" />

          <ol className="timeline">
            {milestones.map(([label, at]) => (
              <li key={label} className={at ? "is-done" : ""}>
                <span>{label}</span>
                <time>{clock(at, true)}</time>
              </li>
            ))}
          </ol>
        </section>
      ) : (
        <section className="ride ride--idle">
          {status === "charging"
            ? `Charging at ${depot?.name ?? "its hub"}.`
            : status === "returning"
              ? `Heading back to ${depot?.name ?? "its hub"}.`
              : status === "maintenance"
                ? "Out of service at the hub."
                : "No rider. Waiting for a dispatch."}
        </section>
      )}

      {!readOnly && (
        <footer className="commands">
          {cab.halted ? (
            <button className="primary" disabled={!!busy} onClick={() => command("resume", () => client.cab.resume(cab.id, {}))}>
              Resume
            </button>
          ) : (
            <button className="warn" disabled={!!busy} onClick={() => command("pull", () => client.cab.pullOver(cab.id, {}))}>
              Pull over
            </button>
          )}
          {status === "available" && (
            <button className="ghost" disabled={!!busy} onClick={() => command("recall", () => client.cab.recall(cab.id, {}))}>
              Recall to hub
            </button>
          )}
          {trip && ["requested", "assigned", "arrived"].includes(trip.status) && (
            <button
              className="ghost"
              disabled={!!busy}
              onClick={() =>
                command("cancel", () =>
                  client.trip.cancel(trip.id, { cancelledAt: new Date().toISOString(), cancelReason: "Cancelled by operator" }),
                )
              }
            >
              Cancel trip
            </button>
          )}
          {["available", "returning", "charging"].includes(status) && (
            <button className="ghost" disabled={!!busy} onClick={() => command("ground", () => client.cab.ground(cab.id, {}))}>
              Take out of service
            </button>
          )}
          {status === "maintenance" && (
            <button className="primary" disabled={!!busy} onClick={() => command("release", () => client.cab.release(cab.id, {}))}>
              Return to service
            </button>
          )}
          {error && <p className="commands__error">{error}</p>}
        </footer>
      )}
    </aside>
  );
}
