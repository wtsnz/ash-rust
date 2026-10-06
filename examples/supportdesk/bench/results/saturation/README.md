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

All four ran on an Apple M4 Max with other applications open (the load average at the start is
in each manifest), so use them as a baseline to compare a later run against on the same
machine, not as capacity figures. The manifests name the commit they ran at, with uncommitted changes: the
driver was committed after the runs.
