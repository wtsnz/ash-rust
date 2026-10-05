defmodule Kanban.Sqlite.Card do
  @moduledoc "Card, on SQLite."
  use Ash.Resource,
    domain: Kanban.Sqlite.Boards,
    data_layer: AshSqlite.DataLayer,
    fragments: [Kanban.Fragments.Card]

  sqlite do
    table "cards"
    repo Kanban.Repo
  end

  relationships do
    belongs_to :list, Kanban.Sqlite.List,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true

    belongs_to :board, Kanban.Sqlite.Board,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true

    has_many :checklist_items, Kanban.Sqlite.ChecklistItem,
      destination_attribute: :card_id,
      public?: true

    has_many :comments, Kanban.Sqlite.Comment,
      destination_attribute: :card_id,
      public?: true
  end
end
