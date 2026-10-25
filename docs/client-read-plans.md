---
read_when:
  - Adding a client query parser or changing relationship filtering
  - Configuring keyset pagination or subscription filters
---

# Client read plans

Client expressions and server preparations enter the core read planner before a data
layer runs them. GraphQL filters and sorts, RPC filters, nested options, `getBy`,
identities and attribute-named read arguments are client input. Hidden fields read as
null in these expressions, including negated comparisons. Server preparations may
filter or sort by hidden fields.

`input::filter_input` and GraphQL's `parse_resource_filter` return
`ClientExpr<Filter>`. Use `into_filter()` to retain that provenance inside a read
request. The sort parsers return `ClientExpr<Vec<Sort>>`; `plan_client_sort` applies
the reader's field visibility. Carry filter provenance into a scoped read so core can
resolve the bound tenant too. The legacy `guard_input_filter` helper plans filters
without tenant context. For explicit planning with a tenant, use `plan_filter` or let the scoped read
resolve `ClientExpr::into_filter()`. Trusted Rust queries still accept `Filter` and `Sort`.
Generated preparation functions and handwritten `PreparationDef::Filter` and
`FilterWithArgs` callbacks return `TrustedExpr::Filter(...)`.

Core resolves each client relationship hop into a `Traversal` with the destination's
default read filter preparations, row policies and tenant predicate. A many-to-many traversal also
contains its join resource's separately scoped condition. Memory and SQL lower those
same conditions. Aggregate destination and join reads use the same core scope builder.
Trusted preparation hops retain their server rules over private related rows. Their
read filter preparations and tenant scope still apply. Core always authorizes the
resource returned by the outer read. The preparation recursion guard remains in core: a default read preparation that
reaches its own resource is applied once inside itself. Excessive recursive expression
expansion returns an error.

`TrustedExpr` is non-exhaustive. Future data-layer-tagged expressions can be added to
it without giving client parsers a backend expression constructor. This release does
not add fragments, full-text operators or locking nodes. The deprecated raw data-layer
APIs remain until the executable-plan migration.

## Sealed keyset cursors

Runtime owns a cursor encryption key. Keyset tokens use RustCrypto
[XChaCha20-Poly1305](https://docs.rs/chacha20poly1305/0.10.1/chacha20poly1305/),
with a fresh random 192-bit nonce for each token. Encryption hides the values;
authentication rejects tampering. Tokens also authenticate the resource, read action,
tenant, complete actor identity and attributes, system authority and effective sort,
including its field guards.

Configure one secret, random 32-byte key at application startup and share it across
instances that serve the same application:

```rust,ignore
let runtime = Runtime::new(data).with_cursor_key(cursor_key_from_config);
let ctx = runtime.bind(actor, tenant);
```

Store the key in the application's secret configuration. Configure it before binding
contexts. Runtime clones, system authority and transactions retain the key. Replacing
the key invalidates existing cursors; clients must restart pagination. There is no
rotation key ring or expiration period in this release.

The default key is cryptographically random and local to that Runtime. It is useful
for development and tests, but is not production key configuration: restarting the
process or constructing a separate Runtime invalidates its cursors. Retain Runtime
across requests and set an explicit shared key in production. Cursor keys never
appear in Runtime or Context debug output.

Core seals the effective sort values before View projection. A sealed cursor's values
are trusted server expressions, so pagination works over a hidden prepared relevance
score or other hidden ordering. Clients still receive null for hidden fields. Nested
RPC pages retain only the sealed token alongside their View, never raw sort values.
Forward and backward cursors use the original effective ordering.

Framework keyset endpoints reject unsealed JSON, base64 JSON, bare IDs, forged tokens
and tokens for another binding. This breaks compatibility with earlier cursors. The
low-level SQL compiler still accepts caller-built `KeysetCursor` data, but now sends
its comparisons and effective sort through core's client visibility rules. It cannot
use that data to make trusted comparisons over a hidden field.

## Subscription snapshots

`Filter::snapshot_match` returns `Match`, `NoMatch` or `NeedsData`. An unloaded
relationship or another missing dependency needs data; it never counts as a match.
A comparison with SQL null remains unknown under negation. Explicit null checks
retain their boolean semantics.

GraphQL validates subscription dependencies before planning can simplify hidden
fields away, then validates the planned filter and default-read visibility predicate
at registration. Filters that need relationships, calculations or aggregates are rejected
with an explicit error. Field-policy conditions used by an attribute filter must also
be evaluable from stored snapshot attributes. Supported attribute filters continue to
apply to create, update and destroy events. This release does not query relationships
in response to events or implement general relational subscription predicates.

Calculation references inherit operand visibility unless explicitly declassified.
Aggregate references project their source operands under each source record's field
policies. Client comparisons and sorts use these projected values; trusted
preparations retain stored-value semantics. See [Derived values](scoped-access.md#derived-values).
Private event payloads, grouped subscription actors and per-event relational
revalidation remain separate migration steps.

## Data-layer-specific expressions

`TrustedExpr::DataLayer` binds a `SqlFragment` defined in developer code. Client
parsers cannot construct one, and client-field planning rejects one smuggled inside
a `ClientExpr<Filter>`. Preparations, calculation dependencies and SQL field aliases
follow the same planning rules as other trusted expressions. See
[Trusted data-layer nodes](trusted-data-layer-nodes.md).

Distinct reads apply authenticated cursor predicates after selecting group winners.
Their sealed cursor binding includes the group fields and fixed winner ordering.
Existing cursor bindings for reads without distinct groups are preserved.
