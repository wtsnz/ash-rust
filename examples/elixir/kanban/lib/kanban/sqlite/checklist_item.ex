defmodule Kanban.Sqlite.ChecklistItem do
  @moduledoc "ChecklistItem, on SQLite."
  use Ash.Resource,
    domain: Kanban.Sqlite.Boards,
    data_layer: AshSqlite.DataLayer,
    fragments: [Kanban.Fragments.ChecklistItem]

  sqlite do
    table "checklist_items"
    repo Kanban.Repo
  end

  relationships do
    belongs_to :card, Kanban.Sqlite.Card,
      define_attribute?: false,
      attribute_writable?: true,
      public?: true
  end
end
