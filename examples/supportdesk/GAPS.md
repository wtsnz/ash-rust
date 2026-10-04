# Gaps the twin found

Building the same desk on ash-rust and on Ash turns up where ash-rust falls short of Ash.
Each entry is a gap, what the desk does about it for now, and its status.

| # | Gap | What the desk does meanwhile | Status |
|---|---|---|---|
| 1 | ash-graphql serves no generic actions, and no managed-relationship inputs, which AshGraphql serves (`action`, `managed_relationships`). | `POST /api/route` and `POST /api/bulk` in both apps. | open |
| 2 | ash-graphql ran every request and subscription with the schema's context: no actor or tenant per request or per socket, and shared subscriptions would have been shared across tenants. AshGraphql takes them from the connection. | — | fixed in this branch: `graphql_router_with` and `RequestData` |
| 3 | Field policies refused writes, which Ash's don't: they govern reads only. | — | fixed in this branch |
| 4 | A typed record can't hold a redacted field that isn't `Option`: redaction yields null, so `R::from_fields` fails on a non-nullable attribute under a field policy. Ash marks the field forbidden whatever its type. | — | fixed: such a field is `Guarded<T>`, forbidden where hidden (the desk keeps `requester_email` nullable, as its Elixir schema has it) |
| 5 | No DSL for an atomic expression update, Ash's `atomic_update(:view_count, expr(view_count + 1))`. | — | fixed: `change atomic_update(field, expr)` |
| 6 | A generic action's `run`, written in the DSL, is generic over any data layer, so it can't open a transaction (`TransactionSupport`). Ash's generic actions take `transaction? true`. | — | fixed: a generic action takes `transaction;`, and runs by name (`Resource::run_generic`) |
| 7 | No AshTypescript RPC: ash-typescript generates a GraphQL client, not AshTypescript's `rpc_action` methods over `/rpc/run`. | — | served in this branch: `ash_typescript::rpc`, wire-compatible with `AshTypescript.Rpc.run_action`. Generating the typed client is still open (15) |
| 8 | A redacted field comes back null with no error. AshGraphql also reports it, as a `forbidden_field` error with the field's path, alongside the null. | `parity` counts AshGraphql's `forbidden_field` errors and leaves them out of the comparison. | open |
| 9 | An action's or calculation's argument was declared as text unless it was a UUID, text, an integer or a boolean: a list or a map argument refused the list or map a client sent. | — | fixed in this branch |
| 10 | An argument takes no default, as Ash's `default: []` does. | — | fixed: `argument name: T [default: expr];` |
| 11 | Validation stops at the first invalid field. Ash reports every one. | — | fixed in this branch: every failure reported (`Error::Multiple`) |
| 12 | An update or destroy by id ignored the read action's policies. AshGraphql and AshTypescript find the record through the read action as the actor (`Ash.bulk_update` over its query), so one the actor can't read is not found, not forbidden. | — | fixed in this branch, for GraphQL and RPC alike |
| 13 | A list attribute or argument (`AttrType::Array`) has no element type, so GraphQL serves a list of maps as `[String]`, where AshGraphql serves `{:array, :map}` as `[Json]`. | — | open (also: no `has` filter operator on lists) |
| 14 | No `/rpc/validate`, AshTypescript's validation of an action's input without running it. | — | fixed in this branch: `Rpc::validate`, `POST /rpc/validate` |
| 15 | ash-typescript doesn't generate AshTypescript's typed RPC client (a function per `rpc_action`). | The benchmark drives both desks through the client the Elixir desk generates, `client/ash_rpc.ts`. | open |
| 16 | Field policies didn't guard a client's filter or sort: a viewer filtering tickets by `requester_email` found them by the hidden value. Ash reads a client's reference to a hidden field as null where it's hidden (`if <policy> then field else nil`). | — | fixed in this branch, for GraphQL and RPC alike |
| 17 | Nulls sorted first ascending in memory and SQLite, last in Postgres, and keyset pages couldn't walk past a null. Ash sorts them last ascending and first descending everywhere, and its keysets step over them. | — | fixed in this branch |
| 18 | Text was stored as given. Ash's string types trim it and read blank text as nil (`trim?`, `allow_empty?` false by default). | — | fixed in this branch (Ash's defaults; the constraints themselves aren't modeled) |
| 19 | Validation messages were ash-rust's own, interpolated. Ash's are templates with vars (`must have length of between %{min} and %{max}`), which AshTypescript sends as they are and AshGraphql fills in. | — | fixed in this branch |
| 20 | A caller's sort replaced the read's prepared sort. Ash appends one to the other: after a code interface's or AshGraphql's sort, before AshTypescript's (which sorts after running the read's preparations). | — | fixed in this branch |
| 21 | An atomic update a policy refused whatever the record was refused before its validations ran. Ash validates a changeset's input first. | — | fixed in this branch |
| 22 | Read actions had no `pagination` declaration. | — | fixed in this branch for the DSL and RPC; ash-graphql still pages every read, where AshGraphql serves an unpaginated action as a list |
| 23 | No action metadata (`metadata :name, :type`, `show_metadata`, `metadataFields`). | — | open |
| 24 | No `field_names`/`argument_names` mappings, nor `typed_query`, AshTypescript's Phoenix channels or client hooks. | — | open (most belong with the client generator, 15) |
| 25 | RPC can't select fields within an embedded resource, typed map or union: they come back whole. | — | open |
| 26 | A float inside a map comes back as text: `Value` has no float. | — | open |
| 27 | Primary keys are UUIDs only. | — | open |
| 28 | ash-rust's SQLite runs no update as one statement: it reads the record first, so concurrent updates of it can lose one another's changes. AshSqlite runs atomic updates. | — | open |

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
- **AshTypescript has no shape for several Ash errors**, an unknown input key, an
  unknown filter field or operator, a malformed UUID, a calculation argument of the
  wrong type among them, and answers each as an `internal_error` with only an id.
  ash-rust says what's wrong (`invalid_argument`, `invalid`,
  `invalid_calculation_args`), so those answers differ on purpose. So does a page
  request a read can't count or that lacks a limit it needs: `invalid_page`, where
  Ash's errors have no shape either.
