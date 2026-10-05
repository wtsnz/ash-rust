defmodule Kanban.Memory.User do
  @moduledoc "User, on ETS."
  use Ash.Resource,
    domain: Kanban.Memory.Workspaces,
    data_layer: Ash.DataLayer.Ets,
    fragments: [Kanban.Fragments.User]
end
