defmodule Cybercab.Fleet.Depot do
  @moduledoc "A Supercharger hub: where cabs charge, and where the shop is."
  use Ash.Resource,
    domain: Cybercab.Fleet,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshGraphql.Resource],
    notifiers: [Ash.Notifier.PubSub]

  postgres do
    table "depots"
    repo Cybercab.Repo
  end

  attributes do
    uuid_primary_key :id
    attribute :code, :string, allow_nil?: false, public?: true
    attribute :name, :string, allow_nil?: false, public?: true
    attribute :lng, :float, allow_nil?: false, public?: true
    attribute :lat, :float, allow_nil?: false, public?: true
    attribute :stalls, :integer, allow_nil?: false, public?: true
  end

  identities do
    identity :unique_code, [:code]
  end

  relationships do
    has_many :cabs, Cybercab.Fleet.Cab, public?: true
  end

  aggregates do
    count :cab_count, :cabs, public?: true

    count :charging, :cabs do
      filter expr(status == :charging)
      public? true
    end
  end

  actions do
    defaults [:read]

    create :open do
      primary? true
      accept [:code, :name, :lng, :lat, :stalls]
    end
  end

  graphql do
    type :depot

    queries do
      get :get_depot, :read
      list :list_depots, :read
    end

    mutations do
      create :open_depot, :open
    end

    subscriptions do
      pubsub CybercabWeb.Endpoint
      subscribe(:depot_created, do: action_types([:create]))
      subscribe(:depot_updated, do: action_types([:update]))
      subscribe(:depot_destroyed, do: action_types([:destroy]))
    end
  end
end
