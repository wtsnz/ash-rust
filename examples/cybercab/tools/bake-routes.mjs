#!/usr/bin/env node
// Bakes driving routes between every pair of places and depots in `server/data/austin.json`
// into `server/data/routes.json`, so the simulation drives real Austin streets without
// calling a routing service at runtime.
//
//   node tools/bake-routes.mjs [--osrm https://router.project-osrm.org]
//
// Routes come from OSRM over OpenStreetMap data (© OpenStreetMap contributors, ODbL).
// The public demo server is for light use: this asks for one route at a time, politely.

import { readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const osrm = process.argv.includes("--osrm")
  ? process.argv[process.argv.indexOf("--osrm") + 1]
  : "https://router.project-osrm.org";

const city = JSON.parse(await readFile(join(root, "server/data/austin.json"), "utf8"));
const stops = [...city.places, ...city.depots];

/** Decodes a precision-5 encoded polyline into [lng, lat] pairs. */
function decode(encoded) {
  const points = [];
  let index = 0, lat = 0, lng = 0;
  while (index < encoded.length) {
    for (const axis of [0, 1]) {
      let shift = 0, result = 0, byte;
      do {
        byte = encoded.charCodeAt(index++) - 63;
        result |= (byte & 0x1f) << shift;
        shift += 5;
      } while (byte >= 0x20);
      const delta = result & 1 ? ~(result >> 1) : result >> 1;
      if (axis === 0) lat += delta; else lng += delta;
    }
    points.push([lng / 1e5, lat / 1e5]);
  }
  return points;
}

/** Encodes [lng, lat] pairs as a precision-5 polyline. */
function encode(points) {
  let out = "", lastLat = 0, lastLng = 0;
  const push = (value) => {
    let v = value < 0 ? ~(value << 1) : value << 1;
    while (v >= 0x20) {
      out += String.fromCharCode((0x20 | (v & 0x1f)) + 63);
      v >>= 5;
    }
    out += String.fromCharCode(v + 63);
  };
  for (const [lng, lat] of points) {
    const la = Math.round(lat * 1e5), ln = Math.round(lng * 1e5);
    push(la - lastLat);
    push(ln - lastLng);
    lastLat = la;
    lastLng = ln;
  }
  return out;
}

/** Douglas–Peucker simplification, tolerance in metres. */
function simplify(points, tolerance) {
  const toMetres = (p) => [p[0] * 96_000, p[1] * 111_000]; // close enough at Austin's latitude
  const distance = (p, a, b) => {
    const [px, py] = toMetres(p), [ax, ay] = toMetres(a), [bx, by] = toMetres(b);
    const dx = bx - ax, dy = by - ay;
    const length = dx * dx + dy * dy;
    const t = length === 0 ? 0 : Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / length));
    return Math.hypot(px - (ax + t * dx), py - (ay + t * dy));
  };
  const keep = new Array(points.length).fill(false);
  keep[0] = keep[points.length - 1] = true;
  const stack = [[0, points.length - 1]];
  while (stack.length) {
    const [first, last] = stack.pop();
    let worst = 0, at = -1;
    for (let i = first + 1; i < last; i++) {
      const d = distance(points[i], points[first], points[last]);
      if (d > worst) [worst, at] = [d, i];
    }
    if (worst > tolerance) {
      keep[at] = true;
      stack.push([first, at], [at, last]);
    }
  }
  return points.filter((_, i) => keep[i]);
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function route(from, to) {
  const coords = `${from.at[0]},${from.at[1]};${to.at[0]},${to.at[1]}`;
  const url = `${osrm}/route/v1/driving/${coords}?overview=full&geometries=polyline`;
  for (let attempt = 1; ; attempt++) {
    try {
      const res = await fetch(url, { headers: { "User-Agent": "ash-rust cybercab example route baker" } });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const body = await res.json();
      if (body.code !== "Ok") throw new Error(body.code);
      const best = body.routes[0];
      return {
        polyline: encode(simplify(decode(best.geometry), 6)),
        distance_m: Math.round(best.distance),
        duration_s: Math.round(best.duration),
      };
    } catch (error) {
      if (attempt >= 5) throw new Error(`${from.code} → ${to.code}: ${error.message}`);
      await sleep(1000 * attempt);
    }
  }
}

const routes = {};
let done = 0;
const total = (stops.length * (stops.length - 1)) / 2;
for (let i = 0; i < stops.length; i++) {
  for (let j = i + 1; j < stops.length; j++) {
    const [a, b] = [stops[i], stops[j]];
    routes[`${a.code}|${b.code}`] = await route(a, b);
    done++;
    if (done % 25 === 0 || done === total) console.log(`  ${done}/${total} routes`);
    await sleep(200);
  }
}

await writeFile(
  join(root, "server/data/routes.json"),
  JSON.stringify({ source: "OSRM over OpenStreetMap (© OpenStreetMap contributors, ODbL)", routes }, null, 0) + "\n",
);
console.log(`wrote server/data/routes.json (${Object.keys(routes).length} routes)`);
