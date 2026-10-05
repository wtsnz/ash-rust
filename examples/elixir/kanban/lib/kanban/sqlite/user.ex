defmodule Kanban.Sqlite.User do
  @moduledoc "User, on SQLite."
  use Ash.Resource,
    domain: Kanban.Sqlite.Workspaces,
    data_layer: AshSqlite.DataLayer,
    fragments: [Kanban.Fragments.User]

  sqlite do
    table "users"
    repo Kanban.Repo
  end
end
