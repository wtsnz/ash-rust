# Saturation results

Each directory is one run of `saturation.ts` (see the [supportdesk README](../../../README.md#mixed-saturation)):
its `report.md`, its `manifest.json` (machine, versions, options, binary hashes) and every
window in `results.jsonl`.

| Run | Basis | Command |
|---|---|---|
| `2026-10-06-common` | both desks offered the same heavy rates, multiples of the slower desk's capacity | `node bench/saturation.ts --fixture /tmp/fixture.json` |
| `2026-10-06-own` | each desk offered multiples of its own capacity | `node bench/saturation.ts --fixture /tmp/fixture.json --basis own --levels 0.5,0.9,1,1.25,1.5,2,3` |

| `2026-10-06-astro-common` | the CPU-bound target (`--target astro`), the same heavy rates for both | `node bench/saturation.ts --target astro` |
| `2026-10-06-astro-own` | the CPU-bound target, multiples of each desk's own capacity | `node bench/saturation.ts --target astro --basis own --levels 0.5,0.9,1,1.25,1.5,2,3` |

Each directory also holds `chart.html` (open it in a browser: hover for every value, a table
view, light and dark) and `chart.svg` (the same charts as one image, for a README), drawn by
`node bench/chart.ts` from `results.jsonl`: per load step, the cheap and heavy requests'
latency, answered a second and failures, then the server's memory and cores, then how the cheap
stream recovers.

| `2026-10-06-pool-matrix` | one 30 s test per pool policy at two heavy rates and two pool sizes, with Postgres sampled | `node bench/saturation.ts --fixture /tmp/fixture.json --matrix` |

| `2026-10-06-equal-limit` | the same wait limit (100, 250, 1,000 ms) for both desks, on 20 connections, at three heavy rates | `node bench/saturation.ts --fixture /tmp/fixture.json --matrix --pools 20 --limits 100,250,1000 --rates 400,800,1600` |

All six ran on an Apple M4 Max with other applications open (the load average at the start is
in each manifest), so use them as a baseline to compare a later run against on the same
machine, not as capacity figures. The manifests name the commit they ran at, with uncommitted changes: the
driver was committed after the runs.
