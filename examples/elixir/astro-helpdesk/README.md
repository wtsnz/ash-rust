# Astro helpdesk, on Ash

The GraphQL server of [`examples/astro-helpdesk`](../../astro-helpdesk) on Elixir Ash,
AshGraphql and Absinthe, on ETS: seeded with the same two representatives and three tickets,
and serving the same API at the same paths, so the Astro frontend and its generated SDK run
against either server.

```bash
mix deps.get
PORT=4000 mix run --no-halt     # GraphQL at /graphql, GraphiQL at /graphiql, /health
```

Then, from the Rust example's `frontend`: `API_URL=http://127.0.0.1:4000 bun run dev`.

`priv/schema.graphql` is the schema AshGraphql generates for the domain, regenerated with:

```bash
mix absinthe.schema.sdl --schema AstroHelpdesk.Schema priv/schema.graphql
```

`examples/astro-helpdesk/tests/schema_parity.rs` checks the Rust server's schema against it:
the same types, fields, arguments, input fields and enum values, with the same nullability.

## Tests

```bash
mix test
```

`test/graphql_test.exs` sends the GraphQL documents of the Rust example's
`tests/integration_test.rs` to the endpoint.

What the twin found missing, or different, is in [`examples/GAPS.md`](../../GAPS.md).
