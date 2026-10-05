defmodule Kanban.Sqlite.Workspace do
  @moduledoc "Workspace, on SQLite."
  use Ash.Resource,
    domain: Kanban.Sqlite.Workspaces,
    data_layer: AshSqlite.DataLayer,
    fragments: [Kanban.Fragments.Workspace]

  sqlite do
    table "workspaces"
    repo Kanban.Repo
  end

  relationships do
    has_many :members, Kanban.Sqlite.WorkspaceMember,
      destination_attribute: :workspace_id,
      public?: true
  end
end
