# Gaps the twin found

Building the same desk on ash-rust and on Ash turns up where ash-rust falls short of Ash.
Each entry is a gap, what the desk does about it for now, and its status.

| # | Gap | What the desk does meanwhile | Status |
|---|---|---|---|
| 1 | ash-graphql serves no generic actions, and no managed-relationship inputs, which AshGraphql serves (`action`, `managed_relationships`). | `POST /api/route` and `POST /api/bulk` in both apps. | open |
| 2 | ash-graphql ran every request and subscription with the schema's context: no actor or tenant per request or per socket, and shared subscriptions would have been shared across tenants. AshGraphql takes them from the connection. | — | fixed in this branch: `graphql_router_with` and `RequestData` |
| 3 | Field policies refused writes, which Ash's don't: they govern reads only. | — | fixed in this branch |
| 4 | A typed record can't hold a redacted field that isn't `Option`: redaction yields null, so `R::from_fields` fails on a non-nullable attribute under a field policy. Ash marks the field forbidden whatever its type. | `requester_email` is nullable in both apps, and `open` requires it. | open |
| 5 | No DSL for an atomic expression update, Ash's `atomic_update(:view_count, expr(view_count + 1))`. | A `CustomChange` with an atomic plan (`changes.rs`). | open |
| 6 | A generic action's `run`, written in the DSL, is generic over any data layer, so it can't open a transaction (`TransactionSupport`). Ash's generic actions take `transaction? true`. | The server supplies `.run(...)`, where the data layer is Postgres. | open |
| 7 | No AshTypescript RPC: ash-typescript generates a GraphQL client, not AshTypescript's `rpc_action` methods over `/rpc/run`. | — | served in this branch: `ash_typescript::rpc`, wire-compatible with `AshTypescript.Rpc.run_action`. Generating the typed client is still open (15) |
| 8 | A redacted field comes back null with no error. AshGraphql also reports it, as a `forbidden_field` error with the field's path, alongside the null. | `parity` counts AshGraphql's `forbidden_field` errors and leaves them out of the comparison. | open |
| 9 | An action's or calculation's argument was declared as text unless it was a UUID, text, an integer or a boolean: a list or a map argument refused the list or map a client sent. | — | fixed in this branch |
| 10 | An argument takes no default, as Ash's `default: []` does. | `comments` is optional in the Rust desk, where the Elixir one defaults it to `[]`. | open |
| 11 | Validation stops at the first invalid field. Ash reports every one. | `parity`'s invalid inputs have one invalid field each. | open |
| 12 | An update or destroy by id ignored the read action's policies. AshGraphql and AshTypescript find the record through the read action as the actor (`Ash.bulk_update` over its query), so one the actor can't read is not found, not forbidden. | — | fixed in this branch, for GraphQL and RPC alike |
| 13 | A list attribute or argument (`AttrType::Array`) has no element type, so GraphQL serves a list of maps as `[String]`, where AshGraphql serves `{:array, :map}` as `[Json]`. | — | open |
| 14 | No `/rpc/validate`, AshTypescript's validation of an action's input without running it. | The generated client doesn't call it (validation functions aren't generated). | open |
| 15 | ash-typescript doesn't generate AshTypescript's typed RPC client (a function per `rpc_action`). | The benchmark drives both desks through the client the Elixir desk generates, `client/ash_rpc.ts`. | open |

## What the twin found in Ash's packages

Behaviours of the Elixir packages the comparison works around. ash-rust doesn't copy them.

- **AshTypescript 0.19.0 reads a relationship selected with options without the actor.**
  A nested selection written as options (`{"comments": {"fields": [...], "sort": ...}}`,
  with any of `filter`, `sort`, `page`, `limit`, `offset`) is built as
  `Ash.Query.for_read(read_action)` with no actor (`field_selector.ex`). Ash skips its
  own `for_read` for an already validated query when loading it, so the parent's actor
  never reaches it: the related resource's read policies see no one, and an agent's, or
  even an admin's, internal comments are filtered out. Under `authorize :when_requested`
  the nested read isn't authorized at all. The plain form (`{"comments": [...]}`) is read
  as the actor. ash-rust reads both as the actor, as AshGraphql does. `parity` reads
  nested comments with options as a viewer, or filtered to public ones.
- **AshTypescript answers an AshStateMachine `NoMatchingTransition` as `internal_error`**,
  having no error protocol implementation for it. ash-rust answers the same.
- **AshTypescript's generated types refuse `count` on a keyset page** (`count?: never`)
  though the action is `countable` and the server counts it.
