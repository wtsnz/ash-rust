// Draws a saturation run: for each load step, what the cheap and the heavy requests got
// (answered, latency, failures) and what the server paid (memory, cores), ash-rust beside Ash.
//
//   node examples/supportdesk/bench/chart.ts [results-dir ...]
//
// With no arguments it draws every run under bench/results/saturation. Each directory gets
// `chart.html` (hover for every value, a table view, light and dark) and `chart.svg` (the
// same charts as one image, for a README). Neither needs anything but a browser.

import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { median } from "./stats.ts";

// What a run recorded -------------------------------------------------------------------

type Stream = { due: number; ok: number; failed: number; timeouts: number; p50: number | null; p99All: number | null; dropped: number };
type Step = {
  kind: "step";
  rep: number;
  side: "rust" | "elixir";
  level: number;
  offered: number;
  cheap: Stream;
  heavy: Stream | null;
  goodput: number;
  server: { cores: number | null; maxRssMiB: number | null };
};
type Recovery = { kind: "recovery"; rep: number; side: "rust" | "elixir"; seconds: Array<{ p99: number | null }>; recoveredAfter: number | null };
type Capacity = { kind: "capacity"; side: "rust" | "elixir"; perSecond: number };

const sides = ["rust", "elixir"] as const;
type Side = (typeof sides)[number];
const names: Record<Side, string> = { rust: "ash-rust", elixir: "Ash (Elixir)" };

// The numbers each chart plots ----------------------------------------------------------

type Point = {
  /** What the x axis is: the heavy rate offered (same basis) or the multiple of the desk's own capacity. */
  x: number;
  level: number;
  offered: number;
  cheapP50: number | null;
  cheapP99: number | null;
  cheapAnswered: number;
  cheapFailedPct: number;
  heavyAnswered: number;
  heavyP50: number | null;
  heavyFailedPct: number;
  heavyNotSentPct: number;
  rssMiB: number | null;
  cores: number | null;
};

const med = (values: Array<number | null | undefined>): number | null => {
  const v = values.filter((x): x is number => typeof x === "number" && Number.isFinite(x));
  return v.length ? median(v) : null;
};

const load = (dir: string) => {
  const manifest = JSON.parse(readFileSync(`${dir}/manifest.json`, "utf8"));
  const rows = readFileSync(`${dir}/results.jsonl`, "utf8").trim().split("\n").map((l) => JSON.parse(l));
  const steps = rows.filter((r) => r.kind === "step") as Step[];
  const recoveries = rows.filter((r) => r.kind === "recovery") as Recovery[];
  const capacities = rows.filter((r) => r.kind === "capacity") as Capacity[];
  const windowSeconds = manifest.options.window / 1000;
  const own = manifest.options.basis === "own";
  const levels = [...new Set(steps.filter((s) => s.level > 0).map((s) => s.level))].sort((a, b) => a - b);
  const baseline = {} as Record<Side, { p50: number | null; p99: number | null }>;
  const series = {} as Record<Side, Point[]>;
  for (const side of sides) {
    const of = (level: number) => steps.filter((s) => s.side === side && s.level === level);
    const base = of(0);
    baseline[side] = { p50: med(base.map((s) => s.cheap.p50)), p99: med(base.map((s) => s.cheap.p99All)) };
    series[side] = levels.map((level) => {
      const at = of(level);
      const heavy = (f: (h: Stream) => number) => med(at.map((s) => (s.heavy ? f(s.heavy) : null))) ?? 0;
      const offered = med(at.map((s) => s.offered)) ?? 0;
      return {
        x: own ? level : offered,
        level,
        offered,
        cheapP50: med(at.map((s) => s.cheap.p50)),
        cheapP99: med(at.map((s) => s.cheap.p99All)),
        cheapAnswered: med(at.map((s) => s.cheap.ok / windowSeconds)) ?? 0,
        cheapFailedPct: med(at.map((s) => (s.cheap.due ? (100 * s.cheap.failed) / s.cheap.due : 0))) ?? 0,
        heavyAnswered: med(at.map((s) => s.goodput)) ?? 0,
        heavyP50: med(at.map((s) => s.heavy?.p50)),
        heavyFailedPct: heavy((h) => (h.due ? (100 * h.failed) / h.due : 0)),
        heavyNotSentPct: heavy((h) => (h.due ? (100 * h.dropped) / h.due : 0)),
        rssMiB: med(at.map((s) => s.server.maxRssMiB)),
        cores: med(at.map((s) => s.server.cores)),
      };
    });
  }
  const seconds = Math.max(...recoveries.map((r) => r.seconds.length));
  const recovery = {} as Record<Side, Array<number | null>>;
  for (const side of sides) {
    const rs = recoveries.filter((r) => r.side === side);
    recovery[side] = Array.from({ length: seconds }, (_, i) => med(rs.map((r) => r.seconds[i]?.p99)));
  }
  const capacity = Object.fromEntries(sides.map((s) => [s, med(capacities.filter((c) => c.side === s).map((c) => c.perSecond))])) as Record<Side, number | null>;
  return { manifest, own, levels, series, baseline, recovery, capacity, reps: new Set(steps.map((s) => s.rep)).size };
};

// Charts --------------------------------------------------------------------------------

type Panel = {
  id: string;
  group: string;
  title: string;
  unit: string;
  scale: "log" | "linear";
  /** The x value of each position, and the label under it. */
  xs: number[];
  xLabels: string[];
  xLabels2?: string[];
  xTitle: string;
  /** Time is linear; load, in doublings, is logarithmic. */
  xScale?: "linear";
  values: Record<Side, Array<number | null>>;
  /** A value as the tooltip and the table say it. */
  fmt: (v: number) => string;
  /** A reference line: [label, value]. */
  ref?: Array<[string, number]>;
  /** The line along which the series would run if nothing were lost: answered = offered. */
  diagonal?: boolean;
  yMax?: number;
};

const num = (v: number, digits = 0) => v.toLocaleString("en-US", { maximumFractionDigits: digits, minimumFractionDigits: digits });
const ms = (v: number) => (v >= 100 ? `${num(v)} ms` : v >= 10 ? `${num(v, 1)} ms` : `${num(v, 2)} ms`);
const pct = (v: number) => `${num(v, v < 10 ? 1 : 0)}%`;
const mem = (v: number) => (v >= 1024 ? `${num(v / 1024, 1)} GiB` : `${num(v)} MiB`);
const perSecond = (v: number) => `${num(v)}/s`;

const panels = (run: ReturnType<typeof load>): Panel[] => {
  const { series, own, levels } = run;
  const first = series.rust;
  const xs = first.map((p) => p.x);
  const xLabels = own ? levels.map((l) => `${l}×`) : first.map((p) => num(p.x));
  const xLabels2 = own ? undefined : levels.map((l) => `${l}×`);
  const xTitle = own ? "heavy load, as a multiple of each desk's own capacity" : "heavy requests offered a second (× the slower desk's capacity)";
  const of = (f: (p: Point) => number | null) => ({ rust: series.rust.map(f), elixir: series.elixir.map(f) });
  const base = { xs, xLabels, xLabels2, xTitle };
  const cheapRate = run.manifest.options.cheapRate as number;
  const cores = run.manifest.machine.cores as number;
  return [
    { ...base, id: "cheap-p50", group: "Cheap request", title: "latency, median", unit: "ms", scale: "log", values: of((p) => p.cheapP50), fmt: ms },
    { ...base, id: "cheap-p99", group: "Cheap request", title: "latency, p99", unit: "ms, a failure counted at the latency it gave up at", scale: "log", values: of((p) => p.cheapP99), fmt: ms },
    { ...base, id: "cheap-answered", group: "Cheap request", title: "answered", unit: `a second, of ${num(cheapRate)} offered; the lines coincide when neither desk fails`, scale: "linear", values: of((p) => p.cheapAnswered), fmt: perSecond, ref: [["offered", cheapRate]], yMax: cheapRate * 1.1 },
    { ...base, id: "cheap-failed", group: "Cheap request", title: "failed", unit: "% of requests due", scale: "linear", values: of((p) => p.cheapFailedPct), fmt: pct, yMax: 100 },
    { ...base, id: "heavy-answered", group: "Heavy request", title: "answered", unit: "a second", scale: "linear", values: of((p) => p.heavyAnswered), fmt: perSecond, diagonal: true },
    { ...base, id: "heavy-p50", group: "Heavy request", title: "latency, median", unit: "ms, of those answered", scale: "log", values: of((p) => p.heavyP50), fmt: ms },
    { ...base, id: "heavy-failed", group: "Heavy request", title: "failed", unit: "% of requests due", scale: "linear", values: of((p) => p.heavyFailedPct), fmt: pct, yMax: 100 },
    { ...base, id: "heavy-notsent", group: "Heavy request", title: "not sent, the driver's limit", unit: "% of requests due, no desk's refusal", scale: "linear", values: of((p) => p.heavyNotSentPct), fmt: pct, yMax: 100 },
    { ...base, id: "memory", group: "Server", title: "memory, peak resident", unit: "MiB", scale: "log", values: of((p) => p.rssMiB), fmt: mem },
    { ...base, id: "cores", group: "Server", title: "CPU, cores busy", unit: `of ${cores}`, scale: "linear", values: of((p) => p.cores), fmt: (v) => `${num(v, 1)} cores`, ref: [[`${cores} cores`, cores]], yMax: cores * 1.1 },
    {
      id: "recovery",
      group: "After the heavy stream stops",
      title: "cheap latency p99, each second",
      unit: "ms",
      scale: "log",
      xs: run.recovery.rust.map((_, i) => i + 1),
      xLabels: run.recovery.rust.map((_, i) => `${i + 1}`),
      xTitle: "seconds since the heavy stream stopped",
      xScale: "linear" as const,
      values: run.recovery,
      fmt: ms,
    },
  ];
};

// Drawing -------------------------------------------------------------------------------

const W = 400;
const H = 244;
const M = { l: 52, r: 58, t: 40, b: 46 };
const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!);
const r1 = (v: number) => Math.round(v * 10) / 10;

const niceLinear = (max: number): { max: number; ticks: number[] } => {
  const raw = max / 4;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 2.5, 5, 10].map((m) => m * mag).find((s) => s >= raw)!;
  const top = Math.ceil(max / step) * step;
  return { max: top, ticks: Array.from({ length: Math.round(top / step) + 1 }, (_, i) => i * step) };
};

/** One chart as an SVG fragment, in its own box of W x H. */
const panelSvg = (panel: Panel, hover: boolean): { svg: string; px: number[] } => {
  const all = sides.flatMap((s) => panel.values[s]).filter((v): v is number => v !== null && v > 0);
  const refs = (panel.ref ?? []).map(([, v]) => v);
  const log = panel.scale === "log";
  let lo = 0;
  let hi = 1;
  let ticks: number[] = [];
  if (log) {
    lo = 10 ** Math.floor(Math.log10(Math.min(...all)));
    hi = 10 ** Math.ceil(Math.log10(Math.max(...all, lo * 10)));
    for (let t = lo; t <= hi * 1.0001; t *= 10) ticks.push(t);
  } else {
    const max = Math.max(...all, ...refs, panel.diagonal ? Math.max(...panel.xs) : 0, 1e-9);
    const nice = niceLinear(panel.yMax ?? max * 1.05);
    lo = 0;
    hi = panel.yMax === 100 ? 100 : nice.max;
    ticks = panel.yMax === 100 ? [0, 25, 50, 75, 100] : nice.ticks;
  }
  const iw = W - M.l - M.r;
  const ih = H - M.t - M.b;
  const xv = panel.xs;
  const place = panel.xScale === "linear" ? (v: number) => v : Math.log;
  const xlo = place(Math.min(...xv));
  const xhi = place(Math.max(...xv));
  const sameX = xv.length < 2 || xhi === xlo;
  const px = xv.map((v) => r1(M.l + (sameX ? iw / 2 : ((place(v) - xlo) / (xhi - xlo)) * iw)));
  const every = Math.ceil(panel.xLabels.length / 8);
  const py = (v: number) => r1(M.t + ih - (log ? (Math.log10(v) - Math.log10(lo)) / (Math.log10(hi) - Math.log10(lo)) : (v - lo) / (hi - lo)) * ih);
  const tickLabel = (v: number) => (v >= 1000 ? num(v) : String(v));
  const out: string[] = [];
  out.push(`<text class="ptitle" x="${M.l - 40}" y="16">${esc(`${panel.group.split(" ")[0]}: ${panel.title}`)}</text>`);
  out.push(`<text class="punit" x="${M.l - 40}" y="31">${esc(panel.unit)}</text>`);
  for (const t of ticks) {
    const y = py(t);
    out.push(`<line class="${t === lo ? "axis" : "grid"}" x1="${M.l}" x2="${W - M.r}" y1="${y}" y2="${y}"/>`);
    out.push(`<text class="tick" x="${M.l - 8}" y="${y + 3.5}" text-anchor="end">${tickLabel(Math.round(t * 100) / 100)}</text>`);
  }
  panel.xLabels.forEach((label, i) => {
    if (i % every !== 0 && i !== panel.xLabels.length - 1) return;
    out.push(`<text class="tick" x="${px[i]}" y="${H - M.b + 15}" text-anchor="middle">${esc(label)}</text>`);
    if (panel.xLabels2) out.push(`<text class="tick2" x="${px[i]}" y="${H - M.b + 27}" text-anchor="middle">${esc(panel.xLabels2[i])}</text>`);
  });
  out.push(`<text class="tick2" x="${M.l + iw / 2}" y="${H - 4}" text-anchor="middle">${esc(panel.xTitle)}</text>`);
  for (const [label, v] of panel.ref ?? []) {
    out.push(`<line class="ref" x1="${M.l}" x2="${W - M.r}" y1="${py(v)}" y2="${py(v)}"/>`);
    out.push(`<text class="tick2" x="${W - M.r + 10}" y="${py(v) - 6}">${esc(label)}</text>`);
  }
  if (panel.diagonal) {
    // Answered = offered, the line a desk that lost nothing would draw.
    const pts = panel.xs.map((v, i) => `${px[i]},${py(v)}`).join(" ");
    out.push(`<polyline class="ref" fill="none" points="${pts}"/>`);
    out.push(`<text class="tick2" x="${W - M.r + 10}" y="${py(Math.min(panel.xs[panel.xs.length - 1], hi)) + 3.5}">offered</text>`);
  }
  const labelYs: Array<[Side, number]> = [];
  sides.forEach((side, k) => {
    const cls = k === 0 ? "s1" : "s2";
    const pts = panel.values[side].map((v, i) => (v === null || (log && v <= 0) ? null : `${px[i]},${py(v)}`));
    const line = pts.filter((p): p is string => p !== null).join(" ");
    if (line.includes(" ")) out.push(`<polyline class="line ${cls}" fill="none" points="${line}"/>`);
    panel.values[side].forEach((v, i) => {
      if (v === null || (log && v <= 0)) return;
      out.push(`<circle class="dot ${cls}" cx="${px[i]}" cy="${py(v)}" r="4"><title>${esc(`${names[side]}: ${panel.fmt(v)} at ${panel.xLabels[i]}`)}</title></circle>`);
    });
    const last = panel.values[side].map((v, i) => [v, i] as const).filter(([v]) => v !== null).pop();
    if (last) labelYs.push([side, py(last[0]!)]);
  });
  // Direct labels at the line ends, only where they don't collide: the legend carries the rest.
  if (labelYs.length === 2 && Math.abs(labelYs[0][1] - labelYs[1][1]) >= 13) {
    for (const [side, y] of labelYs) out.push(`<text class="end" x="${W - M.r + 6}" y="${y + 3.5}">${side === "rust" ? "Rust" : "Elixir"}</text>`);
  }
  if (hover) out.push(`<line class="cross" x1="0" x2="0" y1="${M.t}" y2="${H - M.b}" visibility="hidden"/><rect class="hit" x="${M.l}" y="${M.t}" width="${iw}" height="${ih}" fill="transparent"/>`);
  return { svg: out.join("\n"), px };
};

// Page ----------------------------------------------------------------------------------

const CSS = `
.viz-root, svg.viz {
  color-scheme: light;
  --surface: #fcfcfb; --page: #f9f9f7; --ink: #0b0b0b; --sec: #52514e; --muted: #898781;
  --grid: #e1e0d9; --axis: #c3c2b7; --s1: #2a78d6; --s2: #eb6834; --ring: rgba(11,11,11,0.10);
}
@media (prefers-color-scheme: dark) {
  :root:where(:not([data-theme="light"])) .viz-root, svg.viz {
    color-scheme: dark;
    --surface: #1a1a19; --page: #0d0d0d; --ink: #ffffff; --sec: #c3c2b7; --muted: #898781;
    --grid: #2c2c2a; --axis: #383835; --s1: #3987e5; --s2: #d95926; --ring: rgba(255,255,255,0.10);
  }
}
:root[data-theme="dark"] .viz-root {
  color-scheme: dark;
  --surface: #1a1a19; --page: #0d0d0d; --ink: #ffffff; --sec: #c3c2b7; --muted: #898781;
  --grid: #2c2c2a; --axis: #383835; --s1: #3987e5; --s2: #d95926; --ring: rgba(255,255,255,0.10);
}
svg text { font-family: system-ui, -apple-system, "Segoe UI", sans-serif; }
.ptitle { fill: var(--ink); font-size: 13px; font-weight: 600; }
.punit, .tick2 { fill: var(--muted); font-size: 10.5px; }
.tick { fill: var(--muted); font-size: 10.5px; font-variant-numeric: tabular-nums; }
.end { fill: var(--sec); font-size: 11px; }
.grid { stroke: var(--grid); stroke-width: 1; }
.axis { stroke: var(--axis); stroke-width: 1; }
.ref { stroke: var(--muted); stroke-width: 1; opacity: .7; }
.cross { stroke: var(--muted); stroke-width: 1; }
.line { stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }
.s1.line { stroke: var(--s1); } .s2.line { stroke: var(--s2); }
.dot { stroke: var(--surface); stroke-width: 2; }
.dot.s1 { fill: var(--s1); } .dot.s2 { fill: var(--s2); }
.surface { fill: var(--surface); }
.sectiontitle { fill: var(--ink); font-size: 15px; font-weight: 600; }
.maintitle { fill: var(--ink); font-size: 20px; font-weight: 600; }
.sub { fill: var(--sec); font-size: 12px; }
`;

const HTML_CSS = `
body { margin: 0; background: var(--page, #f9f9f7); font-family: system-ui, -apple-system, "Segoe UI", sans-serif; }
:root { --page-fallback: #f9f9f7; }
.viz-root { background: var(--page); color: var(--ink); padding: 28px clamp(16px, 4vw, 48px) 48px; min-height: 100vh; box-sizing: border-box; }
h1 { font-size: 24px; margin: 0 0 6px; font-weight: 600; }
h2 { font-size: 16px; margin: 30px 0 10px; font-weight: 600; }
.lede { color: var(--sec); max-width: 82ch; line-height: 1.5; margin: 0 0 14px; font-size: 14px; }
.legend { display: flex; gap: 20px; align-items: center; color: var(--sec); font-size: 13px; margin: 14px 0 4px; flex-wrap: wrap; }
.key { display: inline-flex; gap: 8px; align-items: center; }
.key i { display: inline-block; width: 18px; height: 0; border-top: 2px solid; border-radius: 2px; }
.key.k1 i { border-color: var(--s1); } .key.k2 i { border-color: var(--s2); }
.kpis { display: flex; flex-wrap: wrap; gap: 12px; margin: 18px 0 0; }
.tile { background: var(--surface); border: 1px solid var(--ring); border-radius: 12px; padding: 14px 18px; min-width: 190px; }
.tile .label { color: var(--sec); font-size: 12.5px; }
.tile .value { font-size: 26px; font-weight: 600; margin-top: 4px; }
.tile .value small { font-size: 12.5px; font-weight: 400; color: var(--sec); margin-left: 4px; }
.grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(380px, 1fr)); gap: 14px; }
.panel { background: var(--surface); border: 1px solid var(--ring); border-radius: 12px; position: relative; padding: 6px 4px 0; }
.panel svg { width: 100%; height: auto; display: block; overflow: visible; }
.tip { position: absolute; pointer-events: none; background: var(--surface); border: 1px solid var(--ring); box-shadow: 0 4px 16px rgba(0,0,0,.18); border-radius: 8px; padding: 8px 10px; font-size: 12px; color: var(--sec); white-space: nowrap; z-index: 2; }
.tip b { color: var(--ink); font-size: 13px; font-weight: 600; margin-right: 6px; }
.tip .row { display: flex; align-items: center; gap: 8px; margin-top: 3px; }
.tip .line { display: inline-block; width: 14px; border-top: 2px solid; }
.tip .head { color: var(--ink); font-weight: 600; margin-bottom: 2px; }
details { margin-top: 28px; }
summary { cursor: pointer; font-size: 14px; color: var(--ink); }
table { border-collapse: collapse; margin-top: 12px; font-size: 12.5px; background: var(--surface); }
th, td { border: 1px solid var(--grid); padding: 4px 10px; text-align: right; font-variant-numeric: tabular-nums; color: var(--ink); }
th { color: var(--sec); font-weight: 500; }
td:first-child, th:first-child { text-align: left; }
.note { color: var(--sec); font-size: 12.5px; line-height: 1.5; max-width: 90ch; margin-top: 18px; }
button.theme { position: fixed; top: 14px; right: 18px; background: var(--surface); color: var(--ink); border: 1px solid var(--ring); border-radius: 8px; padding: 6px 12px; font-size: 12px; cursor: pointer; }
`;

const TIP_JS = `
const panels = JSON.parse(document.getElementById('data').textContent);
for (const fig of document.querySelectorAll('.panel')) {
  const p = panels[fig.dataset.id];
  const svg = fig.querySelector('svg');
  const tip = fig.querySelector('.tip');
  const cross = svg.querySelector('.cross');
  const hit = svg.querySelector('.hit');
  const show = (e) => {
    const box = svg.getBoundingClientRect();
    const scale = ${W} / box.width;
    const x = (e.clientX - box.left) * scale;
    let best = 0;
    p.px.forEach((v, i) => { if (Math.abs(v - x) < Math.abs(p.px[best] - x)) best = i; });
    cross.setAttribute('x1', p.px[best]); cross.setAttribute('x2', p.px[best]); cross.setAttribute('visibility', 'visible');
    tip.replaceChildren();
    const head = document.createElement('div'); head.className = 'head'; head.textContent = p.xTitle2[best]; tip.append(head);
    p.series.forEach((s, k) => {
      const row = document.createElement('div'); row.className = 'row';
      const key = document.createElement('span'); key.className = 'line'; key.style.borderColor = 'var(--s' + (k + 1) + ')';
      const v = document.createElement('b'); v.textContent = s.text[best];
      const n = document.createElement('span'); n.textContent = s.name;
      row.append(key, v, n); tip.append(row);
    });
    tip.hidden = false;
    const fb = fig.getBoundingClientRect();
    let left = e.clientX - fb.left + 14;
    if (left + tip.offsetWidth > fb.width - 4) left = e.clientX - fb.left - tip.offsetWidth - 14;
    tip.style.left = left + 'px'; tip.style.top = (e.clientY - fb.top + 14) + 'px';
  };
  const hide = () => { tip.hidden = true; cross.setAttribute('visibility', 'hidden'); };
  hit.addEventListener('pointermove', show); hit.addEventListener('pointerleave', hide);
}
document.querySelector('button.theme').addEventListener('click', () => {
  const root = document.documentElement;
  const dark = root.dataset.theme ? root.dataset.theme === 'dark' : matchMedia('(prefers-color-scheme: dark)').matches;
  root.dataset.theme = dark ? 'light' : 'dark';
});
`;

const title = (run: ReturnType<typeof load>) => {
  const o = run.manifest.options;
  const astro = run.manifest.target === "astro";
  return {
    heading: astro ? "Mixed saturation, CPU-bound (in memory)" : "Mixed saturation (over Postgres)",
    sub: `${new Date(run.manifest.started).toISOString().slice(0, 10)}, ${run.reps} reps, medians. ${astro ? `Cheap: { __typename }, ${o.cheapRate}/s. Heavy: a sort, filter and count over ${o.tickets} tickets, a page of 25.` : `Cheap: one ticket by id, ${o.cheapRate}/s. Heavy: the 250 newest tickets with their relationships.`} ${o.basis === "own" ? "Each desk is offered multiples of its own capacity." : "Both desks are offered the same heavy rate."} ${run.manifest.machine.cpu}, ${run.manifest.machine.cores} cores.`,
  };
};

const tableHtml = (run: ReturnType<typeof load>, ps: Panel[]) => {
  const head = `<tr><th>${esc(ps[0].xTitle)}</th>${ps
    .filter((p) => p.id !== "recovery")
    .flatMap((p) => sides.map((s) => `<th>${esc(p.group.split(" ")[0])} ${esc(p.title)}, ${s === "rust" ? "Rust" : "Elixir"}</th>`))
    .join("")}</tr>`;
  const body = ps[0].xs
    .map((_, i) => `<tr><td>${esc(ps[0].xLabels[i])}${ps[0].xLabels2 ? ` (${esc(ps[0].xLabels2[i])})` : ""}</td>${ps
      .filter((p) => p.id !== "recovery")
      .flatMap((p) => sides.map((s) => `<td>${p.values[s][i] === null ? "-" : esc(p.fmt(p.values[s][i]!))}</td>`))
      .join("")}</tr>`)
    .join("");
  const rec = ps.find((p) => p.id === "recovery")!;
  const recHead = `<tr><th>Second after the heavy stream stops</th><th>Rust cheap p99</th><th>Elixir cheap p99</th></tr>`;
  const recBody = rec.xs.map((x, i) => `<tr><td>${x}</td>${sides.map((s) => `<td>${rec.values[s][i] === null ? "-" : esc(rec.fmt(rec.values[s][i]!))}</td>`).join("")}</tr>`).join("");
  return `<table>${head}${body}</table><table>${recHead}${recBody}</table>`;
};

const buildHtml = (run: ReturnType<typeof load>): string => {
  const ps = panels(run);
  const t = title(run);
  const groups = [...new Set(ps.map((p) => p.group))];
  const data: Record<string, unknown> = {};
  const figures = (group: string) =>
    ps
      .filter((p) => p.group === group)
      .map((p) => {
        const { svg, px } = panelSvg(p, true);
        data[p.id] = {
          px,
          xTitle2: p.xLabels.map((l, i) => (p.id === "recovery" ? `${l} s after` : p.xLabels2 ? `${l} heavy/s (${p.xLabels2[i]})` : `${l} of its capacity`)),
          series: sides.map((s) => ({ name: names[s], text: p.values[s].map((v) => (v === null ? "-" : p.fmt(v))) })),
        };
        return `<figure class="panel" data-id="${p.id}" style="margin:0"><svg viewBox="0 0 ${W} ${H}" role="img" aria-label="${esc(`${p.group}: ${p.title} (${p.unit}), ash-rust and Ash`)}">${svg}</svg><div class="tip" hidden></div></figure>`;
      })
      .join("");
  const cap = run.capacity;
  const worst = (side: Side) => run.series[side][run.series[side].length - 1];
  const tile = (label: string, value: string, small = "") => `<div class="tile"><div class="label">${esc(label)}</div><div class="value">${esc(value)}<small>${esc(small)}</small></div></div>`;
  return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>${esc(t.heading)}</title>
<style>${CSS}${HTML_CSS}</style></head>
<body><div class="viz-root">
<button class="theme" type="button">Light / dark</button>
<h1>${esc(t.heading)}</h1>
<p class="lede">${esc(t.sub)}</p>
<div class="legend"><span class="key k1"><i></i>ash-rust</span><span class="key k2"><i></i>Ash (Elixir)</span></div>
<div class="kpis">
${tile("Heavy capacity alone, ash-rust", `${num(cap.rust ?? 0)}`, "a second")}
${tile("Heavy capacity alone, Ash", `${num(cap.elixir ?? 0)}`, "a second")}
${tile("Cheap p99 at the top step, ash-rust", ms(worst("rust").cheapP99 ?? 0))}
${tile("Cheap p99 at the top step, Ash", ms(worst("elixir").cheapP99 ?? 0))}
</div>
${groups.map((g) => `<h2>${esc(g)}</h2><div class="grid">${figures(g)}</div>`).join("\n")}
<details><summary>Table view</summary>${tableHtml(run, ps)}</details>
<p class="note">Latency counts from when a request was due, so a desk that falls behind is charged for the wait. Heavy requests the driver would hold past its limit of ${run.manifest.options.maxInFlight} are not sent; that limit, not a desk, is what the “not sent” chart shows, and it bounds how long a desk that queues can queue. ${run.manifest.target === "astro" ? "" : "Postgres, both desks and the driver share one machine. "}Run on a laptop with other applications open: read it as a baseline.</p>
</div>
<script type="application/json" id="data">${JSON.stringify(data)}</script>
<script>${TIP_JS}</script>
</body></html>`;
};

const buildSvg = (run: ReturnType<typeof load>): string => {
  const ps = panels(run);
  const t = title(run);
  const cols = 3;
  const gap = 12;
  const pad = 20;
  const top = 92;
  const rows = Math.ceil(ps.length / cols);
  const width = pad * 2 + cols * W + (cols - 1) * gap;
  const height = top + rows * (H + gap) + 10;
  const cells = ps
    .map((p, i) => {
      const x = pad + (i % cols) * (W + gap);
      const y = top + Math.floor(i / cols) * (H + gap);
      return `<g transform="translate(${x} ${y})"><rect class="surface" width="${W}" height="${H}" rx="12"/>${panelSvg(p, false).svg}</g>`;
    })
    .join("\n");
  return `<svg xmlns="http://www.w3.org/2000/svg" class="viz" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" role="img" aria-label="${esc(t.heading)}: cheap and heavy request latency, failures, memory and CPU, ash-rust and Ash">
<style>${CSS}svg.viz{background:var(--page)}</style>
<rect width="${width}" height="${height}" fill="var(--page)"/>
<text class="maintitle" x="${pad}" y="32">${esc(t.heading)}</text>
<text class="sub" x="${pad}" y="52">${esc(t.sub.slice(0, 190))}</text>
<line class="s1 line" x1="${pad}" x2="${pad + 20}" y1="72" y2="72" stroke="var(--s1)"/><text class="sub" x="${pad + 28}" y="76">ash-rust</text>
<line class="s2 line" x1="${pad + 110}" x2="${pad + 130}" y1="72" y2="72" stroke="var(--s2)"/><text class="sub" x="${pad + 138}" y="76">Ash (Elixir)</text>
${cells}
</svg>
`;
};

// A pool matrix: each configuration's one test at each heavy rate -------------------------

type Arm = {
  label: string;
  side: Side;
  pool: number;
  rate: number;
  cheap: Stream;
  heavy: Stream | null;
  goodput: number;
  server: { cores: number | null; maxRssMiB: number | null };
  db?: { active: number | null; connections: number | null; containerCpuPct: number | null } | null;
};

const matrixCols: Array<{ title: string; unit: string; value: (a: Arm) => number | null; fmt: (v: number) => string; max?: number }> = [
  { title: "Cheap latency, p50", unit: "ms", value: (a) => a.cheap.p50, fmt: ms },
  { title: "Cheap latency, p99", unit: "ms, p99", value: (a) => a.cheap.p99All, fmt: ms },
  { title: "Cheap failed", unit: "% of requests due", value: (a) => (a.cheap.due ? (100 * a.cheap.failed) / a.cheap.due : 0), fmt: pct, max: 100 },
  { title: "Heavy latency, p50", unit: "ms, of those answered", value: (a) => a.heavy?.p50 ?? null, fmt: ms },
  { title: "Heavy latency, p99", unit: "ms, errors counted", value: (a) => a.heavy?.p99All ?? null, fmt: ms },
  { title: "Heavy failed", unit: "% of requests due", value: (a) => (a.heavy && a.heavy.due ? (100 * a.heavy.failed) / a.heavy.due : 0), fmt: pct, max: 100 },
  { title: "Heavy answered", unit: "a second", value: (a) => a.goodput, fmt: perSecond },
  { title: "Memory, peak", unit: "resident", value: (a) => a.server.maxRssMiB, fmt: mem },
];

/** One heavy rate's bars: a row for each configuration, a column for each measure. */
const matrixSvgFor = (arms: Arm[], rate: number, hover: boolean): string => {
  const rowH = 30;
  const labelW = 260;
  const colW = 150;
  const top = 62;
  const pools = [...new Set(arms.map((a) => a.pool))];
  const rows: Array<Arm | { gap: number }> = pools.flatMap((pool) => [{ gap: pool }, ...arms.filter((a) => a.pool === pool)]);
  const height = top + rows.length * rowH + 18;
  const width = labelW + matrixCols.length * colW + 16;
  const out: string[] = [`<text class="sectiontitle" x="8" y="20">${num(rate)} heavy requests a second offered</text>`];
  matrixCols.forEach((col, c) => {
    const x0 = labelW + c * colW;
    out.push(`<text class="ptitle" x="${x0}" y="40">${esc(col.title)}</text><text class="punit" x="${x0}" y="54">${esc(col.unit)}</text>`);
    const max = col.max ?? Math.max(...arms.map((a) => col.value(a) ?? 0), 1e-9);
    let y = top;
    for (const row of rows) {
      if ("gap" in row) {
        if (c === 0) out.push(`<text class="sec" x="8" y="${y + 20}" style="font-size:12px;font-weight:600;fill:var(--ink)">${row.gap} connections</text>`);
      } else {
        const v = col.value(row);
        const barMax = colW - 66;
        if (c === 0) {
          out.push(`<rect x="8" y="${y + 9}" width="12" height="12" rx="2" style="fill:var(--${row.side === "rust" ? "s1" : "s2"})"/><text class="sec" x="28" y="${y + 19.5}" style="font-size:12px;fill:var(--sec)">${esc(row.label)}</text>`);
        }
        if (v !== null) {
          const w = Math.max(2, (v / max) * barMax);
          const tip = `${row.label}, ${row.pool} connections, ${num(rate)}/s: ${col.title.toLowerCase()} ${col.fmt(v)}`;
          out.push(`<g><title>${esc(tip)}</title><path d="M${x0},${y + 4} h${Math.max(0, w - 4)} a4,4 0 0 1 4,4 v10 a4,4 0 0 1 -4,4 h-${Math.max(0, w - 4)} z" style="fill:var(--${row.side === "rust" ? "s1" : "s2"})"/><text class="tick" x="${x0 + w + 6}" y="${y + 17.5}" style="font-size:11px;fill:var(--sec)">${esc(col.fmt(v))}</text></g>`);
        }
      }
      y += rowH;
    }
  });
  // Configurations that didn't run at this rate.
  return `<g>${out.join("\n")}</g><!--h:${height}w:${width}-->`;
};

const buildMatrix = (dir: string) => {
  const manifest = JSON.parse(readFileSync(`${dir}/manifest.json`, "utf8"));
  const arms = readFileSync(`${dir}/results.jsonl`, "utf8").trim().split("\n").map((l) => JSON.parse(l)).filter((r) => r.kind === "arm") as Arm[];
  const rates = [...new Set(arms.map((a) => a.rate))].sort((a, b) => a - b);
  const o = manifest.options;
  const heading = o.limits ? "Equal wait limit: ash-rust and Ash given the same limit on waiting for a connection" : "Pool matrix: how each pool policy treats overload";
  const sub = `${new Date(manifest.started).toISOString().slice(0, 10)}, one ${o.window / 1000} s test per configuration and rate. Cheap: one ticket by id, ${o.cheapRate}/s. Heavy: the 250 newest tickets with their relationships. ${manifest.machine.cpu}, ${manifest.machine.cores} cores.`;
  const sections = rates.map((rate) => {
    const here = arms.filter((a) => a.rate === rate);
    const body = matrixSvgFor(here, rate, true);
    const [, h, w] = body.match(/<!--h:(\d+)w:(\d+)-->/)!.map(Number) as unknown as number[];
    return { rate, body: body.replace(/<!--.*?-->/, ""), h, w, here };
  });
  // A configuration the memory guard stopped is said so in the image, where it's missing.
  for (const x of sections) {
    const names = new Set(x.here.map((a) => `${a.label}|${a.pool}`));
    const gone = [...new Set(arms.map((a) => `${a.label}|${a.pool}`))].filter((n) => !names.has(n)).map((n) => n.replace("|", ", ") + " connections");
    if (gone.length) {
      x.body += `<text class="punit" x="8" y="${x.h + 6}">Not run at ${num(x.rate)}/s: ${esc(gone.join("; "))}, past ${o.maxRssGiB} GiB resident at the lower rate.</text>`;
      x.h += 18;
    }
  }
  const missing = (rate: number, here: Arm[]) => {
    const names = new Set(here.map((a) => `${a.label}|${a.pool}`));
    const all = [...new Set(arms.map((a) => `${a.label}|${a.pool}`))];
    return all.filter((x) => !names.has(x)).map((x) => x.replace("|", ", ") + " connections");
  };
  const notes = (rate: number, here: Arm[]) => {
    const m = missing(rate, here);
    return m.length ? `Not run at ${num(rate)}/s: ${m.join("; ")}, which had passed ${o.maxRssGiB} GiB resident at the lower rate.` : "";
  };
  const legend = `<span class="key k1"><i></i>ash-rust</span><span class="key k2"><i></i>Ash (Elixir)</span>`;
  const table = (here: Arm[]) =>
    `<table><tr><th>Configuration</th><th>Pool</th>${matrixCols.map((c) => `<th>${esc(c.title)}</th>`).join("")}</tr>${here
      .map((a) => `<tr><td>${esc(a.label)}</td><td>${a.pool}</td>${matrixCols.map((c) => `<td>${c.value(a) === null ? "-" : esc(c.fmt(c.value(a)!))}</td>`).join("")}</tr>`)
      .join("")}</table>`;
  const html = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>${esc(heading)}</title>
<style>${CSS}${HTML_CSS}svg.m{width:100%;height:auto;display:block;background:var(--surface);border:1px solid var(--ring);border-radius:12px;margin-top:14px}</style></head>
<body><div class="viz-root">
<button class="theme" type="button">Light / dark</button>
<h1>${esc(heading)}</h1>
<p class="lede">${esc(sub)}</p>
<div class="legend">${legend}</div>
${sections.map((x) => `<svg class="m" viewBox="0 0 ${x.w} ${x.h}" role="img" aria-label="${esc(`${heading}, ${num(x.rate)} heavy requests a second`)}">${x.body}</svg><p class="note">${esc(notes(x.rate, x.here))}</p>`).join("\n")}
<details><summary>Table view</summary>${sections.map((x) => `<h2>${num(x.rate)}/s</h2>${table(x.here)}`).join("")}</details>
<p class="note">${o.limits ? "Both desks are given the same limit on how long a request waits for a pool connection: Rust's wait_timeout fails a statement that has waited that long; Ecto drops what has waited past twice its queue_target once its pool has been slow, so Elixir's queue_target is half the limit." : "Rust either queues at its pool without limit (the default) or fails a statement that has waited 100 ms; Elixir either sheds, as Ecto's queue_target of 50 ms does (the default), or has queue_target set to 60 s so it queues."} One rep each, on a laptop with other applications open: read the differences between configurations as indications. Heavy requests the driver would hold past its limit of ${o.maxInFlight} are not counted as failed.</p>
</div><script>document.querySelector('button.theme').addEventListener('click',()=>{const r=document.documentElement;const d=r.dataset.theme?r.dataset.theme==='dark':matchMedia('(prefers-color-scheme: dark)').matches;r.dataset.theme=d?'light':'dark'})</script></body></html>`;
  const pad = 20;
  const gap = 24;
  const width = Math.max(...sections.map((x) => x.w)) + pad * 2;
  let y = 96;
  const groups = sections
    .map((x) => {
      const g = `<g transform="translate(${pad} ${y})"><rect class="surface" width="${x.w}" height="${x.h}" rx="12"/>${x.body}</g>`;
      y += x.h + gap;
      return g;
    })
    .join("\n");
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" class="viz" viewBox="0 0 ${width} ${y}" width="${width}" height="${y}" role="img" aria-label="${esc(heading)}">
<style>${CSS}svg.viz{background:var(--page)}</style>
<rect width="${width}" height="${y}" fill="var(--page)"/>
<text class="maintitle" x="${pad}" y="32">${esc(heading)}</text>
<text class="sub" x="${pad}" y="52">${esc(sub.slice(0, 200))}</text>
<rect x="${pad}" y="68" width="12" height="12" rx="2" style="fill:var(--s1)"/><text class="sub" x="${pad + 18}" y="78">ash-rust</text>
<rect x="${pad + 90}" y="68" width="12" height="12" rx="2" style="fill:var(--s2)"/><text class="sub" x="${pad + 108}" y="78">Ash (Elixir)</text>
${groups}
</svg>
`;
  return { html, svg };
};

// Run -----------------------------------------------------------------------------------

const here = fileURLToPath(new URL(".", import.meta.url));
const given = process.argv.slice(2);
const root = `${here}results/saturation`;
const dirs = given.length
  ? given
  : readdirSync(root, { withFileTypes: true })
      .filter((d) => d.isDirectory())
      .map((d) => `${root}/${d.name}`);
for (const dir of dirs) {
  if (!existsSync(`${dir}/results.jsonl`)) continue;
  if (JSON.parse(readFileSync(`${dir}/manifest.json`, "utf8")).options.matrix) {
    const { html, svg } = buildMatrix(dir);
    writeFileSync(`${dir}/chart.html`, html);
    writeFileSync(`${dir}/chart.svg`, svg);
    console.log(`${dir}/chart.html, chart.svg`);
    continue;
  }
  const run = load(dir);
  writeFileSync(`${dir}/chart.html`, buildHtml(run));
  writeFileSync(`${dir}/chart.svg`, buildSvg(run));
  console.log(`${dir}/chart.html, chart.svg`);
}
