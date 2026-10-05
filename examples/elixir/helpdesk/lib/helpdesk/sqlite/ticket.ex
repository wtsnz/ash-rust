defmodule Helpdesk.Sqlite.Ticket do
  @moduledoc "A customer's request for help."
  use Ash.Resource,
    domain: Helpdesk.Sqlite,
    data_layer: AshSqlite.DataLayer,
    fragments: [Helpdesk.TicketFragment]

  sqlite do
    table "tickets"
    repo Helpdesk.Repo
  end

  relationships do
    belongs_to :representative, Helpdesk.Sqlite.Representative,
      public?: true,
      attribute_writable?: true
  end
end
