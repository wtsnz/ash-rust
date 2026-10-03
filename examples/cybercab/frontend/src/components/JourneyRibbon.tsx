import type { Journey } from "../lib/model";
import { duration } from "../lib/geo";

/**
 * A trip's journey as one ribbon: the cab's approach (sensor cyan), the wait at the curb
 * (curb white), and the ride (champagne), each as long as it took or is expected to take.
 * What's behind the cab is solid; what's ahead is a faint track; a bright tick marks
 * where it is now.
 */
export function JourneyRibbon({
  journey,
  size = "row",
}: {
  journey: Journey;
  size?: "row" | "detail" | "wall";
}) {
  if (journey.phase === "hailing") {
    return (
      <div className={`ribbon ribbon--${size} ribbon--hailing`} role="img" aria-label={`Hailing for ${duration(journey.hail)}`}>
        <div className="ribbon__track">
          <div className="ribbon__shimmer" />
        </div>
        {size === "detail" && <div className="ribbon__legend"><span>Finding a cab · {duration(journey.hail)}</span></div>}
      </div>
    );
  }
  if (journey.phase === "cancelled") {
    return (
      <div className={`ribbon ribbon--${size} ribbon--cancelled`} role="img" aria-label="Cancelled">
        <div className="ribbon__track" />
      </div>
    );
  }

  const legs = [
    { key: "approach", label: "Approach", ...journey.approach },
    { key: "curb", label: "Curb", ...journey.curb },
    { key: "ride", label: "Ride", ...journey.ride },
  ];
  // Every leg stays visible, even a short wait at the curb.
  const floor = [0.12, 0.05, 0.2];
  const total = legs.reduce((sum, leg) => sum + leg.seconds, 0) || 1;
  const raw = legs.map((leg, i) => Math.max(leg.seconds / total, floor[i]));
  const scale = raw.reduce((a, b) => a + b, 0);
  const widths = raw.map((w) => w / scale);
  const current = { approach: 0, curb: 1, ride: 2, done: 3 }[journey.phase as "approach" | "curb" | "ride" | "done"];
  let marker = 0;
  widths.forEach((width, i) => {
    if (i < current) marker += width;
    else if (i === current) marker += width * legs[i].done;
  });

  return (
    <div
      className={`ribbon ribbon--${size} ribbon--${journey.phase}`}
      role="img"
      aria-label={`${legs.map((leg) => `${leg.label} ${duration(leg.seconds)}`).join(", ")}`}
    >
      <div className="ribbon__track">
        {legs.map((leg, i) => (
          <div
            key={leg.key}
            className={`ribbon__leg ribbon__leg--${leg.key} ${i < current ? "is-done" : i === current ? "is-live" : ""}`}
            style={{ flexGrow: widths[i] }}
          >
            <div className="ribbon__fill" style={{ width: `${(i < current ? 1 : i === current ? leg.done : 0) * 100}%` }} />
          </div>
        ))}
        {journey.phase !== "done" && <div className="ribbon__now" style={{ left: `${marker * 100}%` }} />}
      </div>
      {size === "detail" && (
        <div className="ribbon__legend">
          {legs.map((leg, i) => (
            <span key={leg.key} className={i === current ? "is-live" : ""}>
              <i className={`dot dot--${leg.key}`} />
              {leg.label} {i <= current || journey.phase === "done" ? duration(leg.seconds) : "—"}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}
