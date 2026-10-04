// Latency summaries, and when a difference between the desks is within noise.

export type Summary = {
  count: number;
  errors: number;
  seconds: number;
  /** Completed requests a second. */
  rate: number;
  p50: number | null;
  p90: number | null;
  /** Reported only from 1,000 samples, as fewer can't place it. */
  p95: number | null;
  /** Reported only from 10,000 samples. */
  p99: number | null;
  max: number | null;
};

const quantile = (sorted: Float64Array, q: number): number =>
  sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))];

/** Milliseconds, rounded to what the clock can tell apart. */
const ms = (value: number): number => Math.round(value * 100) / 100;

export const summarize = (latencies: Float64Array, errors: number, seconds: number): Summary => {
  const sorted = latencies.slice().sort();
  const n = sorted.length;
  const at = (q: number, min: number) => (n >= min ? ms(quantile(sorted, q)) : null);
  return {
    count: n,
    errors,
    seconds,
    rate: Math.round((n / seconds) * 10) / 10,
    p50: at(0.5, 1),
    p90: at(0.9, 100),
    p95: at(0.95, 1_000),
    p99: at(0.99, 10_000),
    max: n ? ms(sorted[n - 1]) : null,
  };
};

export const median = (values: number[]): number => {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
};

/**
 * How the desks compare on one measure, higher or lower being better: the ratio of their
 * medians, and whether it's within noise — within 5%, or within the spread of either
 * desk's reps, whichever is larger.
 */
export const compare = (rust: number[], elixir: number[], higherIsBetter: boolean) => {
  const [r, e] = [median(rust), median(elixir)];
  const spread = (values: number[]) => {
    const m = median(values);
    return m ? (Math.max(...values) - Math.min(...values)) / m : 0;
  };
  const noise = Math.max(0.05, spread(rust), spread(elixir));
  const ratio = higherIsBetter ? r / e : e / r;
  return { rust: r, elixir: e, ratio, withinNoise: Math.abs(ratio - 1) <= noise };
};
