defmodule Supportdesk.Desk.Comment do
  @moduledoc "A reply on a ticket, or an `internal` note only agents see."
  use Ash.Resource,
    domain: Supportdesk.Desk,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshGraphql.Resource, AshTypescript.Resource],
    authorizers: [Ash.Policy.Authorizer]

  postgres do
    table "comments"
    repo Supportdesk.Repo
  end

  multitenancy do
    strategy :attribute
    attribute :org
  end

  attributes do
    uuid_primary_key :id, writable?: true
    attribute :org, :string, allow_nil?: false, public?: true
    attribute :author_id, :uuid, public?: true
    attribute :body, :string, allow_nil?: false, public?: true
    attribute :internal, :boolean, allow_nil?: false, default: false, public?: true
    create_timestamp :inserted_at, public?: true, writable?: true
    update_timestamp :updated_at, public?: true, writable?: true
  end

  relationships do
    belongs_to :ticket, Supportdesk.Desk.Ticket, allow_nil?: false, public?: true, attribute_writable?: true
    belongs_to :author, Supportdesk.Desk.Agent, define_attribute?: false, public?: true
  end

  policies do
    bypass actor_attribute_equals(:role, "admin") do
      authorize_if always()
    end

    policy action_type(:read) do
      authorize_if actor_attribute_equals(:role, "agent")
      authorize_if expr(internal == false)
    end

    policy action_type(:create) do
      authorize_if actor_attribute_equals(:role, "agent")
    end
  end

  actions do
    read :read do
      primary? true
      pagination keyset?: true, countable: true, required?: false
    end

    create :create do
      primary? true
      accept [:ticket_id, :body, :internal]
      change set_attribute(:author_id, actor(:id))
    end

    create :seed do
      accept [:id, :org, :ticket_id, :author_id, :body, :internal, :inserted_at, :updated_at]
    end
  end

  typescript do
    type_name "Comment"
  end

  graphql do
    type :comment

    queries do
      get :get_comment, :read
      list :list_comments, :read
    end

    mutations do
      create :create_comment, :create
    end
  end
end
