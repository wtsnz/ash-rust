# helpdesk

A customer support ticket tracking domain built on **ash-rust**.

Demonstrates resource authorization policies, actor roles (customer vs. representative), calculations, aggregates, generic manual intake actions, and Criterion performance benchmarks.

---

## Domain Overview

Modeled with the `Helpdesk` bounded context via `domain!`:

- **`Ticket` Resource**:
  - Actions: `open`, `read`, `assign`, `close`, `analyze_subject`, `intake`.
  - Policies:
    - Customers can only read and close their own opened tickets.
    - Representatives can view unassigned tickets and assigned tickets, and close assigned tickets.
    - A read no policy lets the reader make (no actor at all) is `Forbidden`, as in Ash; one that depends on the row (another customer's ticket) is filtered away at query time via compiled read filters.
  - Calculations: `subject_length` computed dynamically or inlined into SQL.
  - Relationships: `belongs_to representative: Representative`.
- **`Representative` Resource**:
  - Support agents assigned to tickets.
  - Aggregates: `assigned_ticket_count`, `closed_ticket_count`.

---

## CLI Usage

```bash
# Run CLI
cargo run -p helpdesk -- --help

# Open a new ticket as a customer
cargo run -p helpdesk -- open "Login button does not respond" --customer-id <UUID>

# List accessible tickets
cargo run -p helpdesk -- list
```

---

## The Elixir twin

The same desk on Ash for Elixir, [`examples/elixir/helpdesk`](../elixir/helpdesk): the same
resources, actions and policies, on ETS and on SQLite, with the tests of this example ported
to ExUnit and its benchmarks run on both. What the twin found missing, or different, is in
[GAPS.md](../GAPS.md).

---

## Running Benchmarks & Tests

Run unit and integration tests:

```bash
cargo test -p helpdesk
```

Run Criterion microbenchmarks comparing against baseline performance:

```bash
cargo bench -p helpdesk
```
