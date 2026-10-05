defmodule Kanban.Sqlite.List do
  @moduledoc "List, on SQLite."
  use Ash.Resource,
    domain: Kanban.Sqlite.Boards,
    data_layer: AshSqlite.DataLayer,
    fragments: [Kanban.Fragments.List]

  sqlite do
    table "lists"
    repo Kanban.Repo
  end

  relationships do
    belongs_to :board, Kanban.Sqlite.Board,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true

    has_many :cards, Kanban.Sqlite.Card,
      destination_attribute: :list_id,
      public?: true
  end
end
