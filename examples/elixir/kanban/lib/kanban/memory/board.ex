defmodule Kanban.Memory.Board do
  @moduledoc "Board, on ETS."
  use Ash.Resource,
    domain: Kanban.Memory.Boards,
    data_layer: Ash.DataLayer.Ets,
    fragments: [Kanban.Fragments.Board]

  # AshSqlite serves no resource aggregates, so only the ETS kanban declares them;
  # ash-rust's SQLite computes them in SQL.
  aggregates do
    count :list_count, :lists, public?: true
    count :card_count, :cards, public?: true

    count :open_card_count, :cards do
      public? true
      filter expr(archived == false)
    end
  end

  relationships do
    has_many :lists, Kanban.Memory.List,
      destination_attribute: :board_id,
      public?: true

    has_many :cards, Kanban.Memory.Card,
      destination_attribute: :board_id,
      public?: true
  end
end
