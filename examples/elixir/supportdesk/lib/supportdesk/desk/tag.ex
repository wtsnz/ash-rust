defmodule Supportdesk.Desk.Tag do
  @moduledoc "A label an org files its tickets under."
  use Ash.Resource,
    domain: Supportdesk.Desk,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshGraphql.Resource]

  postgres do
    table "tags"
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
  end

  identities do
    identity :unique_name, [:name]
  end

  actions do
    read :read do
      primary? true
      pagination keyset?: true, countable: true, required?: false
    end

    create :seed do
      accept [:id, :org, :name]
    end
  end

  graphql do
    type :tag

    queries do
      get :get_tag, :read
      list :list_tags, :read
    end
  end
end
