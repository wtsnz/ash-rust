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
| 7 | No AshTypescript RPC: ash-typescript generates a GraphQL client, not AshTypescript's `rpc_action` methods over `/rpc/run`. | — | phase 2 of the benchmark plan |
