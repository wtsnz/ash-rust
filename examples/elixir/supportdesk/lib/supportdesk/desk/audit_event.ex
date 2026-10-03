defmodule Supportdesk.Desk.AuditEvent do
  @moduledoc "Something that happened to a ticket, and who did it."
  use Ash.Resource,
    domain: Supportdesk.Desk,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshGraphql.Resource]

  postgres do
    table "audit_events"
    repo Supportdesk.Repo
  end

  multitenancy do
    strategy :attribute
    attribute :org
  end

  attributes do
    uuid_primary_key :id
    attribute :org, :string, allow_nil?: false, public?: true
    attribute :ticket_id, :uuid, allow_nil?: false, public?: true
    attribute :actor_id, :uuid, public?: true
    attribute :kind, :string, allow_nil?: false, public?: true
    create_timestamp :inserted_at, public?: true
    update_timestamp :updated_at, public?: true
  end

  actions do
    read :read do
      primary? true
      pagination keyset?: true, countable: true, required?: false
    end

    create :record do
      primary? true
      accept [:ticket_id, :kind]
      change set_attribute(:actor_id, actor(:id))
    end
  end

  graphql do
    type :audit_event

    queries do
      get :get_audit_event, :read
      list :list_audit_events, :read
    end
  end
end
