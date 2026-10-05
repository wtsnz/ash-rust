defmodule Helpdesk.Memory.Ticket do
  @moduledoc "A customer's request for help."
  use Ash.Resource,
    domain: Helpdesk.Memory,
    data_layer: Ash.DataLayer.Ets,
    fragments: [Helpdesk.TicketFragment]

  relationships do
    belongs_to :representative, Helpdesk.Memory.Representative,
      public?: true,
      attribute_writable?: true
  end
end
