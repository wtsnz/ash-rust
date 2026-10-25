---
read_when:
  - Adding SQL expressions, full-text search, row locks or distinct reads
  - Implementing a data layer or changing cursor compilation
---

# Trusted data-layer nodes

Developer code can add SQL expressions to a scoped read without executing a separate
query. The planner still applies the read action's preparations, row policies and
tenancy. Returned records go through the same `View` presentation and field policies.

## SQL fragments

A `SqlFragment` lists the data layers that can execute it and a static sequence of
`FragmentPart` values. `Sql` holds developer-written SQL, `Field` names a stored
attribute on the current resource, and `Param` names a typed bound value. Memory
rejects every SQL fragment. A fragment listing only `DataLayerKind::Postgres` also
fails on SQLite with `Error::UnsupportedPlanNode` before acquiring a connection.

```rust,ignore
use ash_core::{AttrType, DataLayerKind, FieldMap, FragmentPart as P, SqlFragment, TrustedExpr};

const MATCH_TITLE: SqlFragment = SqlFragment::new(
    &[DataLayerKind::Postgres, DataLayerKind::Sqlite],
    &[
        P::Sql("lower("), P::Field("title"), P::Sql(") = lower("),
        P::Param { name: "term", ty: AttrType::String }, P::Sql(")"),
    ],
);

fn matching_title(arguments: &FieldMap) -> TrustedExpr {
    MATCH_TITLE.bind(arguments.clone())
}

// In a resource's read action:
// argument term: String;
// prepare trusted_filter(matching_title);
```

`bind` returns `TrustedExpr::DataLayer`. A handwritten `PreparationDef::Filter` or
`FilterWithArgs` can return it. Use `into_filter()` to compose it with other trusted
filters. No client parser accepts fragment SQL, data-layer tags or fragment parts.
The client-field planner also rejects a fragment carried inside `ClientExpr<Filter>`.

Parameters must exist and match their declared Ash type, including typed nulls. Every
occurrence creates a bound placeholder. Values never become SQL text. Put SQL
operators, punctuation and fixed SQL configuration such as a text-search dictionary
in `Sql` parts. Do not put request values, table names, column names or aliases there.
`Field` resolves the actual column through the SQL compiler's current resource and
alias, including correlated and lateral queries. Table selection continues through
the compiler's schema-tenancy rules. A fragment is one SQL expression, not a script
or a general SQL query builder. Developer SQL remains trusted code; subqueries written
inside `Sql` are the developer's responsibility.

Use a fragment calculation for expression sorts:

```rust,ignore
const TITLE_LENGTH: SqlFragment = SqlFragment::new(
    &[DataLayerKind::Postgres, DataLayerKind::Sqlite],
    &[P::Sql("length("), P::Field("title"), P::Sql(")")],
);
// calculations { title_length: Option<i64> = fragment(TITLE_LENGTH); }
// read ranked { prepare sort(title_length, desc); }
```

Handwritten definitions use `Expr::Fragment(fragment)`. Calculation parameters come
from `calc_with_args`, as with `Expr::Arg`. Declare them in the calculation's argument
list too. A `Field` part is a dependency for step 5's visibility rules: a calculation
using a hidden field is hidden unless it declares `declassify`. Explicit policies on
the calculation still apply. Prepared sorts may use hidden results. Runtime-sealed
cursors capture their actual sort values before presentation.

Postgres full-text search uses the same parts. A predicate can combine
`to_tsvector('english', <Field>) @@ plainto_tsquery('english', <Param>)`.
A calculation using `ts_rank` supplies the prepared relevance sort. Pass the search
term both to the preparation's read argument and the calculation's arguments.
The Postgres behavior tests cover hidden relevance and sealed forward paging.

## Row locks

`query.for_update()` requests `ReadLock::ForUpdate`. `query.lock(...)` also accepts
`ForUpdateSkipLocked` and `ForUpdateNowait`. `ReadRequest::lock` exposes the same
operations for scope-bound dynamic reads. No lock SQL comes from client parsers.

Every lock requires a real backend transaction, opened through `Context::transaction`
or `Runtime::transaction`. A lock outside one returns `Error::LockRequiresTransaction`
before querying. Postgres tracks the owning transaction lifetime, so a cloned handle
retained after commit or rollback cannot acquire a lock. It rechecks that state while
holding the connection, before sending SQL. Postgres holds the lock until commit or rollback. `SkipLocked`
omits rows held by another transaction; `Nowait` lets Postgres return its lock error
immediately. A notification buffer does not count as a transaction.

SQLite and memory return `Error::UnsupportedPlanNode` inside a transaction. SQLite
has database write locking rather than Postgres row locks; accepting the request as
a no-op would promise exclusion that it does not provide. Locks cannot be combined
with `distinct_on`, and a count cannot request a row lock.

## Distinct reads and cursors

On Postgres, `query.distinct_on(["category"])` selects one record per stored-field
group. The read's effective sort chooses the winner; core appends the stable primary
key when needed. SQL prefixes the winner ordering with the group fields to satisfy
Postgres `DISTINCT ON`. The outer read uses the requested sort and pagination.
`ReadRequest::distinct_on` takes `DistinctOn { fields, sort }` when developer code
needs a separate winner ordering. An empty `sort` uses the effective read sort.
SQLite and memory reject this node before reading data.

Tenancy, preparation filters and row policies select candidates before grouping.
A keyset boundary applies after grouping, so later pages cannot promote a losing row
from an already returned group. Backward paging reverses the page order while keeping
the winner ordering fixed. Counts count groups. Field policies still project the
selected record through `View`.

The typed query API binds sealed cursors to the distinct fields and winner sort as
well as the existing reader, tenant, action and effective sort. Low-level callers
must pair `seal_keyset_with_distinct` with `keyset_filter_with_distinct` using the same
resolved distinct plan. The older methods remain for reads without distinct groups.
Unsealed SQL cursor compilation rejects a distinct query with a cursor.

## Naming references

AshPostgres documents [fragments](https://hexdocs.pm/ash_postgres/expressions.html)
as SQL expressions that can also define calculations. Ash's
[`lock`](https://hexdocs.pm/ash/Ash.Query.html#lock/2) requires a transaction, and
[`distinct_sort`](https://hexdocs.pm/ash/Ash.Query.html#distinct_sort/3) separates
winner selection from result ordering. The Rust API uses typed lock variants and
`distinct_on` to name the Postgres operation explicitly. The
[AshPagify text-search guide](https://hexdocs.pm/ash_pagify/AshPagify.Tsearch.html)
shows text-search vectors, queries and ranking calculations built from fragments.
