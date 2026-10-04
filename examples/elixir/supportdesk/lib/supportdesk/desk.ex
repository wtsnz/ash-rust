defmodule Supportdesk.Desk do
  @moduledoc "The support desk: orgs, their agents and tags, and the tickets they work."
  use Ash.Domain, extensions: [AshGraphql.Domain, AshTypescript.Rpc]

  typescript_rpc do
    resource Supportdesk.Desk.Ticket do
      rpc_action :list_tickets, :read
      rpc_action :list_noted_tickets, :noted
      rpc_action :get_ticket, :read, get_by: [:id]
      rpc_action :open_ticket, :open
      rpc_action :assign_ticket, :assign
      rpc_action :start_ticket, :start
      rpc_action :hold_ticket, :hold
      rpc_action :resolve_ticket, :resolve
      rpc_action :reopen_ticket, :reopen
      rpc_action :close_ticket, :close
      rpc_action :view_ticket, :view
      rpc_action :edit_ticket, :edit
      rpc_action :destroy_ticket, :destroy
      rpc_action :route_ticket, :route
    end

    resource Supportdesk.Desk.Comment do
      rpc_action :list_comments, :read
      rpc_action :create_comment, :create
    end

    resource Supportdesk.Desk.Agent do
      rpc_action :list_agents, :read
    end

    resource Supportdesk.Desk.Tag do
      rpc_action :list_tags, :read
    end

    resource Supportdesk.Desk.AuditEvent do
      rpc_action :list_audit_events, :read
    end
  end

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
