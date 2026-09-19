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
    - Missing or unauthorized actors are blocked at query time via compiled read filters.
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

## Running Benchmarks & Tests

Run unit and integration tests:

```bash
cargo test -p helpdesk
```

Run Criterion microbenchmarks comparing against baseline performance:

```bash
cargo bench -p helpdesk
```
