# Cybercab Command Center, in Elixir

The [Cybercab Command Center](../../cybercab) built on Ash for Elixir, the way an Ash
developer would build it, so the two can be compared like for like (see
[`examples/benchmarks/cybercab`](../../benchmarks/cybercab)).

The domain mirrors the Rust example's resource for resource: the same four domains
(Fleet, Riders, Rides, Telemetry), the same eight resources, and the same attributes,
actions, validations, identities, aggregates, calculations and state machines, on
`AshPostgres`, `AshStateMachine` and `AshGraphql`. GraphQL queries, mutations and
subscriptions are declared with the names ash-rust gives them, so the two APIs line up.

`priv/schema.graphql` is the schema AshGraphql generates for the domain, regenerated with:

```bash
mix absinthe.schema.sdl --schema Cybercab.Schema priv/schema.graphql
```

It's the reference ash-rust's GraphQL converges on.
