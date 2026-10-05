defmodule Kanban.Sqlite.Comment do
  @moduledoc "Comment, on SQLite."
  use Ash.Resource,
    domain: Kanban.Sqlite.Boards,
    data_layer: AshSqlite.DataLayer,
    fragments: [Kanban.Fragments.Comment]

  sqlite do
    table "comments"
    repo Kanban.Repo
  end

  relationships do
    belongs_to :card, Kanban.Sqlite.Card,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true
  end
end
