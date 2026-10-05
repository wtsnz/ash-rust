defmodule Kanban.Memory.WorkspaceMember do
  @moduledoc "WorkspaceMember, on ETS."
  use Ash.Resource,
    domain: Kanban.Memory.Workspaces,
    data_layer: Ash.DataLayer.Ets,
    fragments: [Kanban.Fragments.WorkspaceMember]

  relationships do
    belongs_to :workspace, Kanban.Memory.Workspace,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true

    belongs_to :user, Kanban.Memory.User,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true
  end
end
