defmodule Kanban.Memory.Card do
  @moduledoc "Card, on ETS."
  use Ash.Resource,
    domain: Kanban.Memory.Boards,
    data_layer: Ash.DataLayer.Ets,
    fragments: [Kanban.Fragments.Card]

  # AshSqlite serves no resource aggregates, so only the ETS kanban declares them;
  # ash-rust's SQLite computes them in SQL.
  aggregates do
    count :checklist_count, :checklist_items, public?: true

    count :completed_checklist_count, :checklist_items do
      public? true
      filter expr(completed == true)
    end

    count :comment_count, :comments, public?: true
  end

  relationships do
    belongs_to :list, Kanban.Memory.List,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true

    belongs_to :board, Kanban.Memory.Board,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true

    has_many :checklist_items, Kanban.Memory.ChecklistItem,
      destination_attribute: :card_id,
      public?: true

    has_many :comments, Kanban.Memory.Comment,
      destination_attribute: :card_id,
      public?: true
  end
end
