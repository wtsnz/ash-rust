defmodule Kanban.Memory.Workspace do
  @moduledoc "Workspace, on ETS."
  use Ash.Resource,
    domain: Kanban.Memory.Workspaces,
    data_layer: Ash.DataLayer.Ets,
    fragments: [Kanban.Fragments.Workspace]

  # AshSqlite serves no resource aggregates, so only the ETS kanban declares them;
  # ash-rust's SQLite computes them in SQL.
  aggregates do
    count :member_count, :members, public?: true
    exists :has_members, :members, public?: true
  end

  relationships do
    has_many :members, Kanban.Memory.WorkspaceMember,
      destination_attribute: :workspace_id,
      public?: true
  end
end
