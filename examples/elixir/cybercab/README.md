# Cybercab Command Center, in Elixir

The [Cybercab Command Center](../../cybercab) built on Ash for Elixir, the way an Ash
developer would build it, so the two can be compared like for like (see
[`examples/benchmarks/cybercab`](../../benchmarks/cybercab)).

The domain mirrors the Rust example's resource for resource: the same four domains
(Fleet, Riders, Rides, Telemetry), the same eight resources, and the same attributes,
actions, validations, identities, aggregates, calculations and state machines, on
`AshPostgres`, `AshStateMachine` and `AshGraphql`. GraphQL queries, mutations and
subscriptions are declared with the names ash-rust gives them, so the two APIs line up:
the Rust example's frontend and generated SDK run against either server.

## Running it

```bash
mix deps.get
DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/cybercab_elixir PORT=4000 mix run --no-halt
```

At startup it migrates the database, empties the command center's tables, seeds Austin
and sets the fleet moving, as the Rust server does. `/health` answers once the city is
seeded. It serves the same paths: GraphQL at `/graphql`, GraphiQL at `/graphiql`,
subscriptions over `graphql-transport-ws` at `/graphql/ws`, and `/metrics`.

It takes the Rust server's settings, from the same variables: `FLEET`, `SIM_SPEED`,
`DEMAND`, `SEED`, and `SIM=off` for a city standing still. One more says how the fleet's
heartbeat is written, which Rust does with a `bulk_update` that Ash has no counterpart
for:

- `HEARTBEAT=concurrent` (the default): each cab's `report` update, run concurrently,
  as many at once as the database pool has connections.
- `HEARTBEAT=upsert`: every cab's report in one `Ash.bulk_create` upsert on the call
  sign, which AshPostgres runs as `INSERT ... ON CONFLICT`. Being a create, each report
  notifies as one: subscribers hear the fleet move on `cabCreated`. The SDK's live
  queries take a create of a record they hold as a replacement, so the frontend follows.

For a release, as the benchmark runs it:

```bash
MIX_ENV=prod mix release
DATABASE_URL=... PORT=4000 _build/prod/rel/cybercab/bin/cybercab start
```

## How it mirrors the Rust example

- **The city.** `lib/cybercab/sim/city.ex` reads the same `austin.json` and baked
  routes from [`examples/shared/cybercab`](../../shared/cybercab).
- **The seed and the simulation.** `seed.ex` and `sim.ex` port the Rust ones step for
  step, drawing from the same SplitMix64 generator (`sim/rng.ex`) in the same order, so a
  seed gives the same city. The simulation is a GenServer ticking once a second, acting
  only through Ash actions.
- **Subscriptions** are AshGraphql's, served through `AshGraphql.Subscription.Endpoint`
  with `AshGraphql.Subscription.Batcher`, as its guide recommends for many subscribers.
  Under load the batcher gathers notifications for up to a second before resolving them.

`priv/schema.graphql` is the schema AshGraphql generates for the domain, regenerated with:

```bash
mix absinthe.schema.sdl --schema Cybercab.Schema priv/schema.graphql
```

It's the reference ash-rust's GraphQL converges on: `examples/cybercab/tests/schema_parity.rs`
checks the Rust server's schema against it.
