defmodule Kanban.Memory.List do
  @moduledoc "List, on ETS."
  use Ash.Resource,
    domain: Kanban.Memory.Boards,
    data_layer: Ash.DataLayer.Ets,
    fragments: [Kanban.Fragments.List]

  # AshSqlite serves no resource aggregates, so only the ETS kanban declares them;
  # ash-rust's SQLite computes them in SQL.
  aggregates do
    count :card_count, :cards, public?: true

    count :open_card_count, :cards do
      public? true
      filter expr(archived == false)
    end

    exists :has_cards, :cards, public?: true
  end

  relationships do
    belongs_to :board, Kanban.Memory.Board,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true

    has_many :cards, Kanban.Memory.Card,
      destination_attribute: :list_id,
      public?: true
  end
end
