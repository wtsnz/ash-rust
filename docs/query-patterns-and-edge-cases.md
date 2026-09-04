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
  * `Filter::and`, `Filter::or`, `Filter::not`, `Filter::eq`, `Filter::ne`, `Filter::gt`, `Filter::gte`, `Filter::lt`, `Filter::lte`, `Filter::in_list`, `Filter::is_nil`.
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

### 4. Inline Subquery Aggregates & Counter Caches

Dashboards frequently require summaries alongside rows (e.g. a list of categories along with their child counts, or whether the current user liked a post).

* **Pattern**:
  ```sql
  SELECT
    "categories"."id",
    "categories"."name",
    (SELECT COUNT(*) FROM "categories" AS "_ash_sub_subcategories_count"
     WHERE "_ash_sub_subcategories_count"."parent_id" = "categories"."id") AS "subcategories_count"
  FROM "categories";
  ```
* **Ash-Rust Solution**:
  * `compile_aggregate` compiles `AggregateKind::Count`, `Exists`, `Sum`, and `First` directly into the `SELECT` projection list as correlated subqueries.
  * Subquery targets are automatically aliased to prevent variable and column shadowing.

---

### 5. Multi-Tenant Scoping & Row-Level Authorization

SaaS applications require strict tenant isolation so data from tenant A never leaks to tenant B.

* **Pattern**:
  * **Schema-based Multi-Tenancy (PostgreSQL)**: Setting the schema search path: `SET LOCAL search_path TO "tenant_1", "public"` or qualifying tables (`"tenant_1"."tickets"`).
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
  * In-memory filtering mirrors SQL null semantics exactly (`null_inequality_identical_in_memory_and_sqlite` test).

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

---

## Part 3: SQLite vs PostgreSQL Dialect Matrix

| Feature / Behavior | SQLite (`ash-sqlite`) | PostgreSQL (`ash-postgres`) |
| :--- | :--- | :--- |
| **Parameter Placeholders** | Positional `?` | Numbered `$1, $2, ...` |
| **`IN` List Compilation** | `IN (SELECT value FROM json_each(?))` | `= ANY($1)` |
| **Boolean Literals** | `1` and `0` (integers) | `TRUE` and `FALSE` |
| **Insert / Update Return** | Two-step (execute + SELECT by PK) | `RETURNING *` (single roundtrip) |
| **UUID Storage** | `TEXT` (36 chars) | Native `UUID` type |
| **JSON Storage** | `TEXT` stringified JSON | Native `JSONB` |
| **Lateral Subqueries** | Window functions / Subqueries | Native `LEFT JOIN LATERAL (...) ON true` |
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
