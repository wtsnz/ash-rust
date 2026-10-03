/** Geometry and formatting helpers for the command center. */

export type LngLat = [number, number];

/** Decodes a precision-5 encoded polyline into [lng, lat] points. */
export function decodePolyline(encoded: string): LngLat[] {
  const points: LngLat[] = [];
  let index = 0;
  let lat = 0;
  let lng = 0;
  while (index < encoded.length) {
    for (const axis of [0, 1]) {
      let shift = 0;
      let result = 0;
      let byte: number;
      do {
        byte = encoded.charCodeAt(index++) - 63;
        result |= (byte & 0x1f) << shift;
        shift += 5;
      } while (byte >= 0x20);
      const delta = result & 1 ? ~(result >> 1) : result >> 1;
      if (axis === 0) lat += delta;
      else lng += delta;
    }
    points.push([lng / 1e5, lat / 1e5]);
  }
  return points;
}

const cache = new Map<string, LngLat[]>();
/** `decodePolyline`, remembered: routes are decoded every frame otherwise. */
export function route(encoded: string | null | undefined): LngLat[] {
  if (!encoded) return [];
  let points = cache.get(encoded);
  if (!points) {
    points = decodePolyline(encoded);
    if (cache.size > 400) cache.clear();
    cache.set(encoded, points);
  }
  return points;
}

/** Metres between two points, flat-earth: plenty at city scale. */
export function metres(a: LngLat, b: LngLat): number {
  const x = (b[0] - a[0]) * 96_000;
  const y = (b[1] - a[1]) * 111_000;
  return Math.hypot(x, y);
}

/**
 * How far along `line` the point nearest `at` is: its share of the line's length,
 * and the line split there into the part behind and the part ahead.
 */
export function progressAlong(line: LngLat[], at: LngLat): { fraction: number; behind: LngLat[]; ahead: LngLat[] } {
  if (line.length < 2) return { fraction: 0, behind: [], ahead: line };
  let best = { distance: Infinity, segment: 0, t: 0 };
  for (let i = 0; i < line.length - 1; i++) {
    const [a, b] = [line[i], line[i + 1]];
    const dx = (b[0] - a[0]) * 96_000;
    const dy = (b[1] - a[1]) * 111_000;
    const px = (at[0] - a[0]) * 96_000;
    const py = (at[1] - a[1]) * 111_000;
    const length = dx * dx + dy * dy;
    const t = length === 0 ? 0 : Math.max(0, Math.min(1, (px * dx + py * dy) / length));
    const distance = Math.hypot(px - t * dx, py - t * dy);
    if (distance < best.distance) best = { distance, segment: i, t };
  }
  let total = 0;
  let travelled = 0;
  for (let i = 0; i < line.length - 1; i++) {
    const length = metres(line[i], line[i + 1]);
    if (i < best.segment) travelled += length;
    if (i === best.segment) travelled += length * best.t;
    total += length;
  }
  const a = line[best.segment];
  const b = line[best.segment + 1];
  const split: LngLat = [a[0] + (b[0] - a[0]) * best.t, a[1] + (b[1] - a[1]) * best.t];
  return {
    fraction: total > 0 ? travelled / total : 0,
    behind: [...line.slice(0, best.segment + 1), split],
    ahead: [split, ...line.slice(best.segment + 1)],
  };
}

/** A circle as a polygon ring, for drawing zones. */
export function circle(center: LngLat, radiusM: number, steps = 64): LngLat[] {
  const ring: LngLat[] = [];
  for (let i = 0; i <= steps; i++) {
    const angle = (i / steps) * Math.PI * 2;
    ring.push([center[0] + (Math.cos(angle) * radiusM) / 96_000, center[1] + (Math.sin(angle) * radiusM) / 111_000]);
  }
  return ring;
}

export const dollars = (cents: number) =>
  `$${(cents / 100).toLocaleString("en-US", { minimumFractionDigits: cents >= 100_000 ? 0 : 2, maximumFractionDigits: cents >= 100_000 ? 0 : 2 })}`;

export const km = (m: number) => `${(m / 1000).toFixed(m < 10_000 ? 1 : 0)} km`;

/** "4m 12s", "38s", "1h 05m". */
export function duration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(s / 3600)}h ${String(Math.floor((s % 3600) / 60)).padStart(2, "0")}m`;
}

export const since = (iso: string | null | undefined, now: number) => (iso ? (now - Date.parse(iso)) / 1000 : 0);

/** "just now", "3m ago", "1h ago". */
export function ago(iso: string | null | undefined, now: number): string {
  const s = since(iso, now);
  if (s < 20) return "just now";
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m ago`;
  return `${Math.round(s / 3600)}h ago`;
}

export const clock = (iso: string | null | undefined, seconds = false) =>
  iso
    ? new Date(iso).toLocaleTimeString("en-US", {
        hour: "numeric",
        minute: "2-digit",
        second: seconds ? "2-digit" : undefined,
        timeZone: "America/Chicago",
      })
    : "—";
