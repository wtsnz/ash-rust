# Relational Query Patterns & Edge Cases in Ash-Rust

This document outlines the most common application data query patterns, notorious edge cases encountered when querying relational databases (PostgreSQL and SQLite), how `ash-rust` (`ash-sql`, `ash-sqlite`, and `ash-postgres`) handles them, and how each is tested.

---

## Part 1: Common Application Query Patterns

### 1. Paginated Feeds & Listings

Applications generally demand two distinct pagination strategies depending on the user experience:

* **Keyset / Cursor Pagination (Feeds, Streams, & Infinite Scroll)**:
  * **Pattern**: `WHERE (created_at < $cursor_time) OR (created_at = $cursor_time AND id < $cursor_id) ORDER BY created_at DESC, id DESC LIMIT 20`.
  * **Rationale**: Offset pagination suffers from $O(N)$ database scan latency and suffers from "page drift" (new rows inserted at the top cause duplicates or skipped items on subsequent pages).
  * **Ash-Rust Solution**: `QueryCompiler::compile_keyset_cursor` translates `KeysetCursor` tuples into deterministic compound order conditions with the primary key as the final tie-breaker.

* **Offset & Limit (Admin Tables & Page Jumpers)**:
  * **Pattern**: `SELECT ... ORDER BY name ASC LIMIT 25 OFFSET 100`.
  * **Rationale**: Necessary when users must jump directly to a specific page number (e.g. page 5 of 50).
  * **Ash-Rust Solution**: `CompiledQuery.limit` and `CompiledQuery.offset` compile directly into dialect-specific `LIMIT ? OFFSET ?` (SQLite) or `LIMIT $1 OFFSET $2` (Postgres).

---

### 2. Multi-Faceted Dashboards & Combinatorial Filters

E-commerce catalogs, issue trackers, and customer portals allow users to filter on any combination of criteria (status, priority, assigned user, date ranges).

* **Pattern**:
  ```sql
  WHERE status = $1
    AND priority >= $2
    AND representative_id IS NULL
    AND category_id = ANY($3)
  ```
* **Ash-Rust Solution**:
  * `Filter::and`, `Filter::or`, `Filter::not`, `Filter::eq`, `Filter::ne`, `Filter::gt`, `Filter::gte`, `Filter::lt`, `Filter::lte`, `Filter::in_list`, `Filter::is_nil`, `Filter::contains`, `Filter::starts_with`, `Filter::ends_with`.
  * Expression operator overloading (`&`, `|`, `!`) allows composing arbitrary filter trees at runtime.
  * Trivial filters (`Filter::True` and `Filter::False`) are simplified (`1=1`, `0=1`).

---

### 3. Graph Preloading & N+1 Prevention (Batch Loading)

When rendering collections of parents with their children (e.g. 50 tickets with their representatives and tags), issuing a query per parent yields the classic $N+1$ query disaster.

* **Pattern**:
  * **PostgreSQL**: `SELECT * FROM comments WHERE post_id = ANY($1)` (single array parameter).
  * **SQLite**: `SELECT * FROM comments WHERE post_id IN (SELECT value FROM json_each(?))` (single JSON array parameter).
* **Ash-Rust Solution**:
  * `ash-graphql` uses `AshBatchLoader` via `DataLoader` to batch-resolve relationship keys.
  * `Filter::In` compiles via `SqlDialect::render_in_list` into high-performance single-parameter statements on both database engines.

---

### 4. Aggregates, Grouped by Relationship

Dashboards frequently require summaries alongside rows (e.g. a list of cabs with how many trips each completed, and what they earned).

* **Pattern**: as ash_sql loads them, a read's aggregates over one relationship share one subquery, each with its own filter, so the related rows are read once however many aggregates there are. The read's records, sorted and paged, are a subquery the aggregates join to.
  * **Lateral** (Postgres, as AshPostgres):
    ```sql
    SELECT "__ash_s".*, "__ash_aggs_trips"."trips_completed", "__ash_aggs_trips"."fares_cents"::bigint
    FROM (SELECT "id", "call_sign", "call_sign" AS "__ash_sort_0" FROM "cabs"
          ORDER BY "__ash_sort_0" ASC LIMIT $1) AS "__ash_s"
    LEFT JOIN LATERAL (
      SELECT COUNT(*) FILTER (WHERE "__ash_agg_trips"."status" = $2) AS "trips_completed",
             SUM("__ash_agg_trips"."fare_cents") FILTER (WHERE "__ash_agg_trips"."status" = $3) AS "fares_cents"
      FROM "trips" AS "__ash_agg_trips" WHERE "__ash_agg_trips"."cab_id" = "__ash_s"."id"
    ) AS "__ash_aggs_trips" ON TRUE
    ORDER BY "__ash_s"."__ash_sort_0" ASC
    ```
  * **Grouped** (SQLite, which has no lateral joins, as AshSqlite): `LEFT JOIN (SELECT key, … GROUP BY key) ON key = record.key`, a record with no related rows counting none.
* **Ash-Rust Solution**:
  * `SqlDialect::aggregate_strategy` chooses, as ash_sql's `aggregate_strategy/1` does.
  * `count`, `exists` and `sum` share their relationship's subquery; `first` is a correlated subquery of its own. A filter or sort that refers to an aggregate writes its subquery inline.
  * The destination's primary read filter (and the join resource's, for a `many_to_many`) applies to the related rows, and every subquery target is aliased, so a relationship back to the same table doesn't shadow it.
  * A sum of no rows is null, as Ash's is, in memory too.

---

### 5. Multi-Tenant Scoping & Row-Level Authorization

SaaS applications require strict tenant isolation so data from tenant A never leaks to tenant B.

* **Pattern**:
  * **Schema-based Multi-Tenancy (PostgreSQL)**: A context-tenant resource's tables are qualified by the tenant's schema in every statement (`"tenant_1"."tickets"`), subqueries included, as AshPostgres prefixes them. The session's `search_path` is never changed.
  * **Row-based Multi-Tenancy**: Appending `WHERE tenant_id = $tenant` to all queries.
* **Ash-Rust Solution**:
  * Every `Context` and `CompiledQuery` carries `tenant: Option<String>`.
  * In `ash-postgres`, queries apply tenant schemas dynamically.
  * In `ash-core`, policies enforce tenant-level and actor-level predicates before any query reaches the data layer.

---

### 6. Idempotent Upserts & Atomic Writes

Webhooks (e.g. Stripe, GitHub) and concurrent ingestion require idempotent writes without race conditions.

* **Pattern**:
  * `INSERT INTO "items" ("id", "sku", "qty") VALUES ($1, $2, $3) ON CONFLICT ("sku") DO UPDATE SET "qty" = EXCLUDED."qty" RETURNING *;`
* **Ash-Rust Solution**:
  * `QueryCompiler::compile_upsert` inspects the resource's `IdentityDef` (unique index columns) and target update attributes, generating dialect-compliant `ON CONFLICT (...) DO UPDATE SET ...`.
  * PostgreSQL emits `RETURNING *` for a 1-network-roundtrip read-after-write.

---

## Part 2: Relational Edge Cases & Pitfalls

### 1. The Empty `IN ()` Clause
* **The Pitfall**: Constructing dynamic SQL where the ID list is empty (e.g. `WHERE id IN ()`). In ANSI SQL, SQLite, and PostgreSQL, `IN ()` with zero items is a syntax error.
* **Ash-Rust Handling**: `Filter::In(_field, vals) if vals.is_empty()` compiles immediately to `0=1` (constant false). A negated empty IN (`NOT (0=1)`) resolves to true.

### 2. SQL 3-Valued Logic & `NULL` Comparison Traps
* **The Pitfall**: In SQL, `NULL = NULL`, `NULL != 'open'`, and `NULL < 5` all evaluate to `UNKNOWN` (falsy in `WHERE` clauses). An untranslated `WHERE status != 'closed'` inadvertently discards all records with `status IS NULL`.
* **Ash-Rust Handling**:
  * `Filter::eq(col, Value::Null)` automatically renders `col IS NULL`.
  * `Filter::ne(col, Value::Null)` automatically renders `col IS NOT NULL`.
  * In-memory filtering and `Filter::matches` mirror SQL null semantics, including `NOT`: `!Filter::contains("email", "x")` leaves out rows whose `email` is null, because `NOT UNKNOWN` is still `UNKNOWN` (`null_inequality_identical_in_memory_and_sqlite` and `text_filters_match_in_memory_and_sqlite` tests).

### 3. Keyset Pagination Sort Drift & Timestamp Collisions
* **The Pitfall**: Paginating on non-unique columns like `created_at DESC`. When multiple records share the identical timestamp (e.g. in bulk operations), cursor `created_at < cursor` drops all other records sharing that exact timestamp.
* **Ash-Rust Handling**: Keyset cursor compilation enforces composite conditions using the primary key `id` as the final tie-breaker:
  `((priority < ?1) OR (priority = ?1 AND id > ?2))`.

### 4. Self-Referential Relational Shadowing
* **The Pitfall**: When a resource relates to itself (e.g. `Comment.parent_id -> Comment.id` or `Category.parent_id -> Category.id`), an unaliased subquery:
  `SELECT COUNT(*) FROM comments WHERE comments.parent_id = comments.id`
  evaluates `comments.parent_id = comments.id` on the *inner* table, comparing a row's parent ID to its own ID and returning `0` everywhere.
* **Ash-Rust Handling**: All subquery targets and join tables are aliased (`AS "_ash_sub_<agg_name>"`, `AS "_ash_join_<agg_name>"`), ensuring the outer table reference remains unambiguous.

### 5. Engine Variable Limits & Parameter Overflow
* **The Pitfall**: SQLite's default variable limit (`SQLITE_LIMIT_VARIABLE_NUMBER` = 999). A query like `WHERE id IN (?, ?, ...)` fails with `too many SQL variables` when querying 1,000+ items. In PostgreSQL, massive `IN ($1, $2, ...)` queries generate enormous ASTs, wasting planner time and defeating prepared statement caching.
* **Ash-Rust Handling**:
  * **SQLite**: Compiles to `IN (SELECT value FROM json_each(?))` with a single JSON array string parameter.
  * **PostgreSQL**: Compiles to `= ANY($1)` with a single native array parameter (`uuid[]`, `text[]`, `bigint[]`).
  * Both databases execute large batches (>1,500 items) in a single parameter with 0 AST bloat.

### 6. Empty Batch & Bulk Writes
* **The Pitfall**: Calling `bulk_create` or `bulk_destroy` with an empty collection. Drivers that blindly issue `DELETE FROM table WHERE id IN ()` fail with syntax errors or corrupt state.
* **Ash-Rust Handling**:
  * `bulk_create`: Returns `Ok(vec![])` immediately without database interaction.
  * `bulk_destroy`: Returns `Ok(())` immediately if the ID slice is empty.
  * `compile_bulk_delete`: Returns `DELETE FROM table WHERE 0=1` when called with empty IDs.

### 7. Optimistic Locking Race Conditions
* **The Pitfall**: Two workers concurrently updating the same record. The second worker overwrites the first worker's modifications without realizing the state changed.
* **Ash-Rust Handling**:
  * If a resource defines an optimistic lock attribute (`version`), `UPDATE` appends `WHERE id = $id AND version = $expected_version`.
  * If rows affected == 0, the driver verifies if the row was deleted (`Error::NotFound`) or modified by another worker (`Error::StaleRecord`).

### 8. Wildcards in Text Search Input
* **The Pitfall**: Building `WHERE title LIKE '%' || $1 || '%'` from a search box. A user typing `50%` or `a_b` gets wildcard matches instead of the literal text, and SQLite's `LIKE` ignores ASCII case while Postgres's does not.
* **Ash-Rust Handling**:
  * `Filter::contains`, `Filter::starts_with`, and `Filter::ends_with` escape the needle, so every character matches literally.
  * Following Ash, `String` fields match case-sensitively and `CiString` fields ignore case. SQLite compiles `String` matches to `GLOB` (case-sensitive) and `CiString` matches to `LIKE ... ESCAPE '\'`, which ignores case for ASCII letters only, so `É` and `é` differ there but not on Postgres or in memory. Postgres compiles both to `LIKE`, and `citext` makes it case-insensitive.
  * Text filters on non-text fields are rejected with an error instead of failing in the database.

---

## Part 3: SQLite vs PostgreSQL Dialect Matrix

| Feature / Behavior | SQLite (`ash-sqlite`) | PostgreSQL (`ash-postgres`) |
| :--- | :--- | :--- |
| **Parameter Placeholders** | Positional `?` | Numbered `$1, $2, ...` |
| **`IN` List Compilation** | `IN (SELECT value FROM json_each(?))` | `= ANY($1)` |
| **Boolean Literals** | `1` and `0` (integers) | `TRUE` and `FALSE` |
| **Insert / Update Return** | Two-step (execute + SELECT by PK) | `RETURNING *` (single roundtrip) |
| **UUID Storage** | `TEXT` (36 chars) | Native `UUID` type |
| **JSON Storage** | `TEXT` holding plain JSON | Native `JSONB` holding plain JSON |
| **IP Addresses (`Inet`)** | `TEXT`, canonical form (`10.0.0.1/32` is stored and matched as `10.0.0.1`) | Native `INET` |
| **Embeddings (`Vector<N>`)** | `TEXT` like `[1,2.5,3]` | pgvector `VECTOR(N)` (migrations create the `vector` extension; the server must have pgvector installed) |
| **Lateral Subqueries** | Window functions / Subqueries | Native `LEFT JOIN LATERAL (...) ON true` |
| **Text Filters** | `GLOB` (`String`), `LIKE ... ESCAPE '\'` (`CiString`) | `LIKE` (`citext` ignores case) |
| **Upsert Syntax** | `ON CONFLICT (...) DO UPDATE SET ...` | `ON CONFLICT (...) DO UPDATE SET ... RETURNING *` |
| **Schema Migrations** | `_ash_schema_migrations` (TEXT) | `_ash_schema_migrations` (VARCHAR) |

---

## Part 4: Test Coverage & Verification

Every pattern and edge case is tested continuously in CI:

1. **Empty IN Predicate**: `crates/ash-sql/tests/compiler_tests.rs` (`test_empty_in_and_not_in_compilation`).
2. **>1,000 Parameter Batch Ingestion**:
   - `crates/ash-sqlite/tests/sqlite_tests.rs` (`test_sqlite_in_query_large_batch_exceeds_1000_limit`).
   - `crates/ash-postgres/tests/postgres_tests.rs` (`test_postgres_in_query_large_batch_any_array`).
3. **Self-Referential Aggregates & Table Shadowing**:
   - `crates/ash-sql/tests/compiler_tests.rs` (`test_self_referential_aggregate_subquery_aliasing`).
   - `crates/ash-sqlite/tests/sqlite_tests.rs` (`test_sqlite_self_referential_aggregate`).
   - `crates/ash-postgres/tests/postgres_tests.rs` (`test_postgres_self_referential_aggregate`).
4. **Keyset Cursor Pagination (Asc/Desc & Composite Sorts)**:
   - `crates/ash-sql/tests/compiler_tests.rs` (`test_keyset_cursor_compilation`).
   - `crates/ash-core/tests/pagination.rs` (`test_keyset_pagination_sqlite`).
5. **Optimistic Locking & Stale Record Rejection**:
   - `crates/ash-core/tests/optimistic_locking.rs` (`test_optimistic_locking_sqlite_increments_and_detects_conflict`).
   - `crates/ash-postgres/tests/postgres_tests.rs` (`test_postgres_optimistic_locking_stale_record`).
6. **SQL 3-Valued Logic & Null Semantics**:
   - `crates/ash-core/tests/null_inequality.rs` (`test_null_inequality_identical_in_memory_and_sqlite`).
7. **Complex Expressions (CASE WHEN, Arithmetic, String Length)**:
   - `crates/ash-sql/tests/compiler_tests.rs` (`test_complex_expressions_compilation`).
8. **Text Search Filters (Wildcards & Case)**:
   - `crates/ash-sql/tests/compiler_tests.rs` (`test_text_filters_escape_wildcards_per_dialect`).
   - `crates/ash-core/tests/text_filters.rs` (`text_filters_match_in_memory_and_sqlite`).
   - `crates/cargo-ash/tests/migrations/queries.rs` (`text_filters_match_literally_and_respect_case`, SQLite and Postgres).
