defmodule Kanban.Sqlite.WorkspaceMember do
  @moduledoc "WorkspaceMember, on SQLite."
  use Ash.Resource,
    domain: Kanban.Sqlite.Workspaces,
    data_layer: AshSqlite.DataLayer,
    fragments: [Kanban.Fragments.WorkspaceMember]

  sqlite do
    table "workspace_members"
    repo Kanban.Repo
  end

  relationships do
    belongs_to :workspace, Kanban.Sqlite.Workspace,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true

    belongs_to :user, Kanban.Sqlite.User,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true
  end
end
