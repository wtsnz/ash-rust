defmodule AstroHelpdesk.Desk do
  @moduledoc "The helpdesk: tickets, and the representatives who write them."
  use Ash.Domain, extensions: [AshGraphql.Domain]

  resources do
    resource AstroHelpdesk.Desk.Ticket
    resource AstroHelpdesk.Desk.Representative
  end
end
