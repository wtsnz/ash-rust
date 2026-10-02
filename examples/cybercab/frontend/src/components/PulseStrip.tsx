import type { AshConnectionStatus, Cab, PulseSample } from "../lib/client";
import { CAB_ORDER, CAB_STATUS, cabStatus } from "../lib/model";
import { dollars, duration } from "../lib/geo";

/** A metric's recent history as a line, newest at the right. */
function Sparkline({ values, color, wide = false }: { values: number[]; color: string; wide?: boolean }) {
  const width = wide ? 112 : 72;
  const height = 22;
  if (values.length < 2) return <svg className="spark" width={width} height={height} />;
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;
  const points = values.map((v, i) => [(i / (values.length - 1)) * width, height - 3 - ((v - min) / span) * (height - 6)]);
  const d = points.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`).join("");
  const [lx, ly] = points[points.length - 1];
  return (
    <svg className="spark" width={width} height={height} viewBox={`0 0 ${width} ${height}`} aria-hidden>
      <path d={`${d}L${width},${height}L0,${height}Z`} fill={color} opacity={0.08} />
      <path d={d} fill="none" stroke={color} strokeWidth={1.4} strokeLinejoin="round" opacity={0.85} />
      <circle cx={lx} cy={ly} r={2.2} fill={color} />
    </svg>
  );
}

function Reading({
  label,
  value,
  unit,
  series,
  color,
  tone,
}: {
  label: string;
  value: string;
  unit?: string;
  series: number[];
  color: string;
  tone?: "warn" | "alarm";
}) {
  return (
    <div className={`reading ${tone ? `reading--${tone}` : ""}`}>
      <div className="reading__label">{label}</div>
      <div className="reading__row">
        <span className="reading__value">
          {value}
          {unit && <small>{unit}</small>}
        </span>
        <Sparkline values={series} color={color} />
      </div>
    </div>
  );
}

const LIVE_LABEL: Record<AshConnectionStatus, string> = {
  idle: "Connecting",
  connecting: "Connecting",
  connected: "Live",
  reconnecting: "Reconnecting",
  closed: "Offline",
};

/**
 * The room's vital signs along the top: the fleet by status, riders aboard and waiting,
 * how long pickups take, how busy the fleet is, and the day's takings, each with the
 * last few minutes behind it.
 */
export function PulseStrip({
  cabs,
  pulse,
  status,
  now,
  wall,
  onToggleWall,
}: {
  cabs: Cab[];
  pulse: PulseSample[];
  status: AshConnectionStatus;
  now: number;
  wall: boolean;
  onToggleWall: () => void;
}) {
  const history = [...pulse].reverse();
  const latest = pulse[0];
  const series = (pick: (p: PulseSample) => number) => history.map(pick);
  const counts = Object.fromEntries(CAB_ORDER.map((s) => [s, cabs.filter((c) => cabStatus(c) === s).length]));
  const inService = cabs.length - (counts.maintenance ?? 0);
  const waiting = latest?.waiting ?? 0;

  return (
    <header className="pulse">
      <div className="pulse__brand">
        <div className="wordmark">
          CYBERCAB<span>OPS</span>
        </div>
        <div className="pulse__place">Austin · Command Center</div>
      </div>

      <div className="fleetbar" aria-label="The fleet by status">
        <div className="fleetbar__head">
          <span>Fleet</span>
          <b>
            {inService}
            <small>/{cabs.length} in service</small>
          </b>
        </div>
        <div className="fleetbar__bar">
          {CAB_ORDER.map((s) =>
            counts[s] ? (
              <div key={s} style={{ flexGrow: counts[s], background: CAB_STATUS[s].color }} title={`${CAB_STATUS[s].label}: ${counts[s]}`} />
            ) : null,
          )}
        </div>
        <div className="fleetbar__legend">
          {CAB_ORDER.filter((s) => counts[s]).map((s) => (
            <span key={s}>
              <i style={{ background: CAB_STATUS[s].color }} />
              {CAB_STATUS[s].short} {counts[s]}
            </span>
          ))}
        </div>
      </div>

      <div className="pulse__readings">
        <Reading label="Riders aboard" value={String(counts.on_trip ?? 0)} series={series((p) => p.onTrip)} color="var(--champagne)" />
        <Reading
          label="Waiting"
          value={String(waiting)}
          series={series((p) => p.waiting)}
          color="var(--sodium)"
          tone={waiting >= 6 ? "alarm" : waiting >= 3 ? "warn" : undefined}
        />
        <Reading label="Avg pickup" value={duration(latest?.avgWaitS ?? 0)} series={series((p) => p.avgWaitS)} color="var(--sensor)" />
        <Reading label="Utilization" value={String(latest?.utilizationPct ?? 0)} unit="%" series={series((p) => p.utilizationPct)} color="var(--curb)" />
        <Reading
          label={`Today · ${latest?.completedToday ?? 0} rides`}
          value={dollars(latest?.revenueCentsToday ?? 0)}
          series={series((p) => p.revenueCentsToday)}
          color="var(--charge)"
        />
      </div>

      <div className="pulse__meta">
        <div className="clock">
          {new Date(now).toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit", second: wall ? undefined : "2-digit", timeZone: "America/Chicago" })}
          <span>CT</span>
        </div>
        <div className={`live live--${status}`} title="Subscription connection">
          <i />
          {LIVE_LABEL[status]}
        </div>
        <button className="ghost" onClick={onToggleWall} title="Toggle wall mode (W)">
          {wall ? "Console" : "Wall"}
        </button>
      </div>
    </header>
  );
}
