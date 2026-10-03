/**
 * The control room's vocabulary: what each status is called and drawn as, and the
 * journey every trip makes (approach, curb, ride), which the journey ribbon draws.
 */
import type { Cab, Trip } from "./ash";
import { progressAlong, route, since, type LngLat } from "./geo";

export const ACTIVE_TRIP_STATES = ["requested", "assigned", "arrived", "riding"] as const;
export const OPEN_ALERT_STATES = ["open", "acknowledged"] as const;

export type CabStatus = "available" | "dispatched" | "on_trip" | "returning" | "charging" | "maintenance";

export const CAB_STATUS: Record<CabStatus, { label: string; short: string; color: string }> = {
  available: { label: "Available", short: "Idle", color: "#c9d4e2" },
  dispatched: { label: "En route to pickup", short: "Pickup", color: "#45c8e8" },
  on_trip: { label: "Carrying a rider", short: "Riding", color: "#d9b97a" },
  returning: { label: "Returning to hub", short: "Return", color: "#8b9db6" },
  charging: { label: "Charging", short: "Charge", color: "#5fd391" },
  maintenance: { label: "Out of service", short: "Service", color: "#4a5466" },
};

export const CAB_ORDER: CabStatus[] = ["on_trip", "dispatched", "available", "returning", "charging", "maintenance"];

export const cabStatus = (cab: Pick<Cab, "status">): CabStatus =>
  (cab.status in CAB_STATUS ? cab.status : "available") as CabStatus;

export const TRIP_STATUS: Record<string, string> = {
  requested: "Hailing",
  assigned: "Cab en route",
  arrived: "At the curb",
  riding: "Riding",
  completed: "Completed",
  cancelled: "Cancelled",
};

export const isActive = (trip: Pick<Trip, "status">) =>
  (ACTIVE_TRIP_STATES as readonly string[]).includes(trip.status);

export type Phase = "hailing" | "approach" | "curb" | "ride" | "done" | "cancelled";

export interface Leg {
  /** Seconds the leg took, or is expected to take. */
  seconds: number;
  /** 0–1: how much of it is behind the cab. */
  done: number;
}

export interface Journey {
  phase: Phase;
  hail: number;
  approach: Leg;
  curb: Leg;
  ride: Leg;
  /** Seconds until the next milestone (pickup, or drop-off), when it can be told. */
  eta?: number;
}

/**
 * Where a trip is in its journey. Finished legs use their recorded times; the leg in
 * progress is measured by how far along its route the cab is, and its length estimated
 * from the time it has taken so far.
 */
export function journeyOf(trip: Trip, cab: Pick<Cab, "lng" | "lat"> | undefined, now: number): Journey {
  const at = (iso?: string | null) => (iso ? Date.parse(iso) : undefined);
  const span = (from?: number, to?: number) => (from !== undefined && to !== undefined ? Math.max(0, (to - from) / 1000) : 0);
  const requested = at(trip.requestedAt)!;
  const assigned = at(trip.assignedAt);
  const arrived = at(trip.arrivedAt);
  const picked = at(trip.pickedUpAt);
  const completed = at(trip.completedAt);
  const position: LngLat | undefined = cab ? [cab.lng, cab.lat] : undefined;

  // The plan until the cab is well along the leg, then what its pace says.
  const estimate = (elapsed: number, fraction: number, planned: number) =>
    fraction > 0.15 || planned <= 0 ? elapsed / Math.max(fraction, 0.02) : Math.max(elapsed, planned);

  const journey: Journey = {
    phase: "hailing",
    hail: span(requested, assigned ?? (trip.status === "requested" ? now : requested)),
    approach: { seconds: span(assigned, arrived), done: arrived ? 1 : 0 },
    curb: { seconds: span(arrived, picked), done: picked ? 1 : 0 },
    ride: { seconds: span(picked, completed), done: completed ? 1 : 0 },
  };

  switch (trip.status) {
    case "requested":
      journey.phase = "hailing";
      break;
    case "assigned": {
      journey.phase = "approach";
      const elapsed = since(trip.assignedAt, now);
      const fraction = position ? progressAlong(route(trip.approachPolyline), position).fraction : 0;
      const planned = span(assigned, at(trip.pickupEtaAt));
      journey.approach = { seconds: estimate(elapsed, fraction, planned), done: fraction };
      journey.eta = Math.max(0, journey.approach.seconds - elapsed);
      break;
    }
    case "arrived":
      journey.phase = "curb";
      journey.curb = { seconds: Math.max(span(arrived, now), 20), done: 0.5 };
      break;
    case "riding": {
      journey.phase = "ride";
      const elapsed = since(trip.pickedUpAt, now);
      const fraction = position ? progressAlong(route(trip.ridePolyline), position).fraction : 0;
      const planned = span(picked, at(trip.dropoffEtaAt));
      journey.ride = { seconds: estimate(elapsed, fraction, planned), done: fraction };
      journey.eta = Math.max(0, journey.ride.seconds - elapsed);
      break;
    }
    case "completed":
      journey.phase = "done";
      break;
    case "cancelled":
      journey.phase = "cancelled";
      journey.hail = span(requested, at(trip.cancelledAt));
      break;
  }
  return journey;
}
