defmodule Supportdesk.Desk.Agent do
  @moduledoc "Someone who works the desk: an `admin`, an `agent`, or a read-only `viewer`."
  use Ash.Resource,
    domain: Supportdesk.Desk,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshGraphql.Resource, AshTypescript.Resource]

  postgres do
    table "agents"
    repo Supportdesk.Repo
  end

  multitenancy do
    strategy :attribute
    attribute :org
  end

  attributes do
    uuid_primary_key :id, writable?: true
    attribute :org, :string, allow_nil?: false, public?: true
    attribute :name, :string, allow_nil?: false, public?: true
    attribute :email, :string, allow_nil?: false, public?: true
    attribute :role, :string, allow_nil?: false, public?: true
    attribute :active, :boolean, allow_nil?: false, default: true, public?: true
  end

  identities do
    identity :unique_email, [:email]
  end

  relationships do
    has_many :assigned_tickets, Supportdesk.Desk.Ticket do
      destination_attribute :assignee_id
      public? true
    end
  end

  aggregates do
    # Open tickets assigned to the agent: what routing balances.
    count :open_assigned, :assigned_tickets do
      filter expr(status == :open)
      public? true
    end
  end

  actions do
    read :read do
      primary? true
      pagination keyset?: true, countable: true, required?: false
    end

    create :seed do
      accept [:id, :org, :name, :email, :role, :active]
    end
  end

  typescript do
    type_name "Agent"
  end

  graphql do
    type :agent

    queries do
      get :get_agent, :read
      list :list_agents, :read
    end
  end
end
