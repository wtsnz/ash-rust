defmodule Helpdesk.Sqlite.Representative do
  @moduledoc "A support agent who takes tickets."
  use Ash.Resource,
    domain: Helpdesk.Sqlite,
    data_layer: AshSqlite.DataLayer,
    fragments: [Helpdesk.RepresentativeFragment]

  sqlite do
    table "representatives"
    repo Helpdesk.Repo
  end

  relationships do
    has_many :tickets, Helpdesk.Sqlite.Ticket,
      destination_attribute: :representative_id,
      public?: true
  end
end
