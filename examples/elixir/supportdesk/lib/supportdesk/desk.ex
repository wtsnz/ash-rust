defmodule Supportdesk.Desk do
  @moduledoc "The support desk: orgs, their agents and tags, and the tickets they work."
  use Ash.Domain, extensions: [AshGraphql.Domain]

  resources do
    resource Supportdesk.Desk.Org
    resource Supportdesk.Desk.Agent
    resource Supportdesk.Desk.Tag
    resource Supportdesk.Desk.Ticket
    resource Supportdesk.Desk.Comment
    resource Supportdesk.Desk.TicketTag
    resource Supportdesk.Desk.AuditEvent
  end
end
