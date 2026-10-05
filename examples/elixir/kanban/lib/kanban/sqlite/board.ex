defmodule Kanban.Sqlite.Board do
  @moduledoc "Board, on SQLite."
  use Ash.Resource,
    domain: Kanban.Sqlite.Boards,
    data_layer: AshSqlite.DataLayer,
    fragments: [Kanban.Fragments.Board]

  sqlite do
    table "boards"
    repo Kanban.Repo
  end

  relationships do
    has_many :lists, Kanban.Sqlite.List,
      destination_attribute: :board_id,
      public?: true

    has_many :cards, Kanban.Sqlite.Card,
      destination_attribute: :board_id,
      public?: true
  end
end
