# Gaps the small twins found

Each of the [helpdesk](helpdesk), [kanban](kanban) and [astro-helpdesk](astro-helpdesk)
examples is built again on Ash for Elixir, in [`elixir/helpdesk`](elixir/helpdesk),
[`elixir/kanban`](elixir/kanban) and [`elixir/astro-helpdesk`](elixir/astro-helpdesk). Writing
the same thing twice shows where ash-rust and Ash differ. The cybercab and supportdesk twins
keep their own list in [`supportdesk/GAPS.md`](supportdesk/GAPS.md).

## Where ash-rust differs from Ash

| # | Gap | What the twins do meanwhile | Status |
|---|---|---|---|
| 1 | A read with no actor at all: ash-rust answered an empty list where Ash answers `Forbidden`, for a read policy that filters on the actor (`expr(opener_id == ^actor(:id))`). Ash's `FilterCheck.strict_check(nil, ...)` is false for a check that references the actor, and a read no policy lets the actor make is `Forbidden`. The same went for an actor whose attributes no `authorize_if` accepts, a `forbid_if` that holds for the actor, `get` by id, and a relationship load whose destination forbids the reader (Ash fails the whole read, with `forbidden` at the relationship's path). | the Elixir test asserts `Forbidden` | fixed: `compile_read_filter` answers `Forbidden` when the policies settle to false before any record is looked at; the tenant a resource needs is still asked for first, as Ash asks. Aggregates and relationship filters still narrow to none, as Ash's do |
| 2 | ash-rust's text filters take `like` and `ilike` on any data layer. AshGraphql serves them only where the data layer defines the functions (AshPostgres does, ETS doesn't). | `astro-helpdesk/tests/schema_parity.rs` treats them as a widening, as it does a state machine's text operators | open: harmless to a client of AshGraphql's |
| 3 | A calculation used only in a filter isn't loaded onto the records in ash-rust, on any data layer. ETS evaluates it onto them; AshSqlite filters in SQL and doesn't. | the Elixir test doesn't assert either way | none needed: Ash's two data layers differ among themselves |

## What the twins found in Ash's packages

Behaviours of the Elixir packages the comparison works around. ash-rust doesn't copy them.

- **AshSqlite (0.2.19) serves no resource aggregates.** `can?({:aggregate, _})` is false, so
  `count`, `exists`, `first` and the rest can't be declared on a resource it stores
  ("`tickets` is not aggregatable"), nor filtered or sorted by. Only the query aggregates
  (`Ash.count/2` and the like) run. ash-rust computes resource aggregates in SQLite, in a
  filter, a sort or a keyset page. The Elixir helpdesk and kanban declare their aggregates
  on the ETS resources alone, and their SQLite resources go without.
- **AshSqlite wraps no action in a transaction unless the repo says it may**
  (`c:AshSqlite.Repo.write_transactions?/0`, false by default), so `Ash.transact` and a
  failed action leave their earlier writes. ash-rust's SQLite always does. The twins' repos
  opt in, and run on one connection so a transaction isn't locked out by another.
- **ETS has no transactions** (`can?(_, :transact)` is false): `Kanban.Multi`'s pipelines roll
  back only on SQLite.
- **AshSqlite has no multitenancy** (`can?(_, :multitenancy)` is false), which the helpdesk
  and kanban don't need but a SQLite supportdesk would.
- **AshGraphql has no GraphQL type for `:binary`**: an attribute of it fails to compile the
  schema. ash-rust serves `Binary` as base64 text (`String`). The Elixir desk keeps the
  base64 text itself, in a type of its own that refuses text that isn't base64, and marks it
  `filterable?: false`, as ash-rust's `Binary` isn't filterable.
- **A code interface refuses a bare `[]` as its last argument**, taking it for options it
  can't tell from params ("Cannot provide an empty list for params"). `Kanban.Templates`
  always passes options.
- **`Ash.transact` turns an `{:error, {step, error}}` into an `Ash.Error.Unknown`**, burying
  the step's name and error. `Kanban.Multi` raises to roll back, and rescues outside.
- **`Ash.DataLayer.Ets.stop/1` can leave a table's manager believing in a table it has
  deleted**, so the next use fails with `:table_not_found` when it's used again from another
  process. The helpdesk benchmark empties the tables in place instead.

## The benchmark that wasn't like for like

`benches/ash_elixir_bench.exs`, which the published Rust-versus-Elixir core figures come from,
modelled a `Ticket` with no policies, while the Rust helpdesk's actions run through its policies
(an actor, a read filter). On the full desk (`elixir/helpdesk/bench/core.exs`, ETS) Elixir took
35.1 µs for `Ticket.open` and 366.6 µs for the filtered read, against the published 26.6 µs and
208.6 µs. The script now carries the desk's policies, and the published core figures are
re-measured with it (`docs/benchmarks.md`, `benches/README.md`): the speedups are 8.1x, 7.4x
and 8.0x, where they were 8.2x, 9.6x and 6.4x.

The Rust Criterion `load_aggregates` benchmark counted nothing: it read as no one, whose read
policy leaves no tickets, and assigned its tickets to a representative who didn't exist. It
counts Bob's twenty tickets as the customer who opened them now, asserts it does, and runs at
26.1 µs where ~3 µs was published; Elixir takes 476 µs for the same query.
