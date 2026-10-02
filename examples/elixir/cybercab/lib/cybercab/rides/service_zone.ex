defmodule Cybercab.Rides.ServiceZone do
  @moduledoc "A part of the city demand is tracked in: riders waiting, the surge, any event on."
  use Ash.Resource,
    domain: Cybercab.Rides,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshGraphql.Resource],
    notifiers: [Ash.Notifier.PubSub]

  postgres do
    table "service_zones"
    repo Cybercab.Repo
  end

  attributes do
    uuid_primary_key :id
    attribute :code, :string, allow_nil?: false, public?: true
    attribute :name, :string, allow_nil?: false, public?: true
    attribute :lng, :float, allow_nil?: false, public?: true
    attribute :lat, :float, allow_nil?: false, public?: true
    attribute :radius_m, :integer, allow_nil?: false, public?: true
    attribute :base_demand, :float, allow_nil?: false, public?: true
    attribute :surge, :float, allow_nil?: false, public?: true
    attribute :waiting, :integer, allow_nil?: false, default: 0, public?: true
    attribute :event_name, :string, public?: true
    attribute :event_boost, :float, allow_nil?: false, public?: true
  end

  identities do
    identity :unique_code, [:code]
  end

  relationships do
    has_many :trips, Cybercab.Rides.Trip, destination_attribute: :zone_id, public?: true
  end

  aggregates do
    count :trips_today, :trips, public?: true

    count :completed_today, :trips do
      filter expr(status == :completed)
      public? true
    end
  end

  actions do
    defaults [:read]

    create :chart do
      primary? true
      accept [:code, :name, :lng, :lat, :radius_m, :base_demand, :surge, :event_boost]
    end

    update :measure do
      primary? true
      accept [:waiting, :surge]
    end

    update :host_event do
      accept [:event_name, :event_boost]
      validate present(:event_name)
      validate numericality(:event_boost, greater_than_or_equal_to: 1, less_than_or_equal_to: 8)
    end

    update :clear_event do
      accept [:event_name, :event_boost]
    end
  end

  graphql do
    type :service_zone

    queries do
      get :get_service_zone, :read
      list :list_service_zones, :read
    end

    mutations do
      create :chart_service_zone, :chart
      update :measure_service_zone, :measure
      update :host_event_service_zone, :host_event
      update :clear_event_service_zone, :clear_event
    end

    subscriptions do
      pubsub CybercabWeb.Endpoint
      subscribe(:service_zone_created, do: action_types([:create]))
      subscribe(:service_zone_updated, do: action_types([:update]))
      subscribe(:service_zone_destroyed, do: action_types([:destroy]))
    end
  end
end
