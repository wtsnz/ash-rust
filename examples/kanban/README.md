# kanban

A full-featured Trello-like multi-domain Kanban application built on **ash-rust**.

Demonstrates modeling complex relational domains, multiple bounded contexts, aggregates, action validations, and multi-step atomic pipelines (`Multi`) with CLI support.

---

## Architecture & Domains

The application is split into two bounded contexts via the `domain!` macro:

1. **`Workspaces` Domain**:
   - `User`: user accounts and registration.
   - `Workspace`: team or organization workspace.
   - `WorkspaceMember`: membership association with role validation (`"admin"`, `"member"`, `"observer"`).
2. **`Kanban` Domain**:
   - `Board`: boards belonging to a workspace with aggregates (`list_count`, `card_count`).
   - `List`: columns belonging to a board, ordered by `position`.
   - `Card`: cards with optimistic locking, labels, position, and aggregates (`comment_count`, `checklist_count`).
   - `Comment`: activity stream and card discussions.
   - `ChecklistItem`: checklist items associated with cards.

---

## CLI Usage

The package includes an interactive CLI (`kanban-cli`):

```bash
# Run CLI with an in-memory database or SQLite
cargo run -p kanban -- --help

# Create a workspace
cargo run -p kanban -- workspace create --name "Acme Corp" --slug "acme"

# Create a board
cargo run -p kanban -- board create --workspace-id <WS_ID> --title "Product Roadmap"

# List boards
cargo run -p kanban -- board list
```

---

## Running Tests

```bash
cargo test -p kanban
```
