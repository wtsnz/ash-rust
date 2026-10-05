defmodule Helpdesk.Memory.Representative do
  @moduledoc "A support agent who takes tickets."
  use Ash.Resource,
    domain: Helpdesk.Memory,
    data_layer: Ash.DataLayer.Ets,
    fragments: [Helpdesk.RepresentativeFragment]

  # AshSqlite serves no resource aggregates (`can?({:aggregate, _})` is false), so only the
  # ETS desk declares them; ash-rust's SQLite computes them in SQL.
  aggregates do
    count :ticket_count, :tickets, public?: true

    count :open_ticket_count, :tickets do
      public? true
      filter expr(status == :open)
    end

    exists :has_tickets, :tickets, public?: true
    first :first_ticket_subject, :tickets, :subject, public?: true
  end

  relationships do
    has_many :tickets, Helpdesk.Memory.Ticket,
      destination_attribute: :representative_id,
      public?: true
  end
end
