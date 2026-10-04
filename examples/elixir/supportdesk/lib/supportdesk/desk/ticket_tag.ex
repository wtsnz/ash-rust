defmodule Supportdesk.Desk.TicketTag do
  @moduledoc "A ticket filed under a tag."
  use Ash.Resource,
    domain: Supportdesk.Desk,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshGraphql.Resource]

  postgres do
    table "ticket_tags"
    repo Supportdesk.Repo

    # Foreign keys, indexed as a real app indexes them (AshPostgres creates none).
    custom_indexes do
      index [:ticket_id]
    end
  end

  multitenancy do
    strategy :attribute
    attribute :org
  end

  attributes do
    uuid_primary_key :id, writable?: true
    attribute :org, :string, allow_nil?: false, public?: true
    attribute :ticket_id, :uuid, allow_nil?: false, public?: true
    attribute :tag_id, :uuid, allow_nil?: false, public?: true
  end

  actions do
    read :read do
      primary? true
      pagination keyset?: true, countable: true, required?: false
    end

    create :seed do
      accept [:id, :org, :ticket_id, :tag_id]
    end
  end

  graphql do
    type :ticket_tag

    queries do
      get :get_ticket_tag, :read
      list :list_ticket_tags, :read
    end
  end
end
