# Helpdesk on Ash

The support desk of [`examples/helpdesk`](../../helpdesk) on Elixir Ash, the way an Ash
developer would write it, so the two can be compared like for like.

The domain mirrors the Rust example's: `Ticket` and `Representative`, the same attributes,
actions (`open`, `assign`, `close`, `analyze_subject`, `intake`, `create`, paged `read`s), the same
calculation (`subject_length`), aggregates, relationships and policies: a customer reads and
closes the tickets they opened, a representative reads the unassigned queue and what is
assigned to them, only a representative assigns.

Each resource is written once, as a Spark fragment (`lib/helpdesk/*_fragment.ex`), and
completed twice, as the Rust resources run on either data layer:

| | Data layer | Rust counterpart |
|---|---|---|
| `Helpdesk.Memory` | `Ash.DataLayer.Ets` | `ash-memory` |
| `Helpdesk.Sqlite` | `AshSqlite.DataLayer` | `ash-sqlite` |

## Tests

```bash
mix deps.get
mix test
```

`test/support/desk_tests.ex` is the Rust example's `tests/helpdesk.rs`, `tests/sqlite.rs` and
`tests/validations.rs` written once and run on both data layers. `test/aggregates_test.exs`
ports `tests/aggregates.rs`, on ETS alone: AshSqlite serves no resource aggregates.

## Benchmarks

```bash
mise exec elixir erlang -- mix run bench/core.exs
```

`bench/core.exs` runs the workloads of the Rust example's `examples/bench.rs` and Criterion suite:
`Ticket.open`, `Representative.create`, a filtered read of 100 tickets, and aggregates, on
ETS and on SQLite, through the desk's policies as the Rust ones run.

What the twin found missing, or different, is in [`examples/GAPS.md`](../../GAPS.md).
