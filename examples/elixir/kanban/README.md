# Kanban on Ash

The boards of [`examples/kanban`](../../kanban) on Elixir Ash: the `Workspaces` domain (`User`,
`Workspace`, `WorkspaceMember`) and the `Boards` domain (`Board`, `List`, `Card`,
`ChecklistItem`, `Comment`), with the same attributes, actions, validations, calculations
and aggregates.

Each resource is written once, as a Spark fragment (`lib/kanban/fragments`), and completed
for ETS (`Kanban.Memory`) and for SQLite (`Kanban.Sqlite`), where it names the data layer and its
relationships. AshSqlite serves no resource aggregates, so only the ETS resources declare them.

Ash has no `Multi`. `Kanban.Multi` runs named steps in order inside one `Ash.transact`, each
given what the steps before it made, and `Kanban.Templates` builds the Rust example's
pipelines with it: the board template, a workspace with its owner, a card moved with a note,
and a card's checklist. ETS has no transactions, so a failed pipeline rolls back only on SQLite
(`Kanban.Repo` opts in to write transactions, which AshSqlite doesn't by default).

## Tests

```bash
mix deps.get
mix test
```

`test/boards_test.exs` and `test/workspaces_test.exs` port the Rust example's
`tests/kanban_board.rs` and `tests/workspaces.rs` on ETS; `test/sqlite_test.exs` ports
`tests/sqlite_cross_domain.rs`, with the rollbacks it checks.

What the twin found missing, or different, is in [`examples/GAPS.md`](../../GAPS.md).
