defmodule Supportdesk.Desk.Org do
  @moduledoc "A customer of the support desk. Its `slug` is the tenant every other resource belongs to."
  use Ash.Resource,
    domain: Supportdesk.Desk,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshGraphql.Resource]

  postgres do
    table "orgs"
    repo Supportdesk.Repo
  end

  attributes do
    uuid_primary_key :id, writable?: true
    attribute :name, :string, allow_nil?: false, public?: true
    attribute :slug, :string, allow_nil?: false, public?: true
  end

  identities do
    identity :unique_slug, [:slug]
  end

  actions do
    read :read do
      primary? true
      pagination keyset?: true, countable: true, required?: false
    end

    create :seed do
      accept [:id, :name, :slug]
    end
  end

  graphql do
    type :org

    queries do
      get :get_org, :read
      list :list_orgs, :read
    end
  end
end
