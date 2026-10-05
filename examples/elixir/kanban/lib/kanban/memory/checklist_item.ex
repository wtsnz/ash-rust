defmodule Kanban.Memory.ChecklistItem do
  @moduledoc "ChecklistItem, on ETS."
  use Ash.Resource,
    domain: Kanban.Memory.Boards,
    data_layer: Ash.DataLayer.Ets,
    fragments: [Kanban.Fragments.ChecklistItem]

  relationships do
    belongs_to :card, Kanban.Memory.Card,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true
  end
end
