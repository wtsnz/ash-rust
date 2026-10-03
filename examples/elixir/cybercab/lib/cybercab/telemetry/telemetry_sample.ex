defmodule Cybercab.Telemetry.TelemetrySample do
  @moduledoc "Where a cab was, how fast and how charged, at a moment: its trail."
  use Ash.Resource,
    domain: Cybercab.Telemetry,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshGraphql.Resource],
    notifiers: [Ash.Notifier.PubSub]

  postgres do
    table "telemetry_samples"
    repo Cybercab.Repo

    custom_indexes do
      index [:cab_id, :recorded_at], name: "telemetry_samples_by_cab_time"
    end
  end

  attributes do
    uuid_primary_key :id
    attribute :recorded_at, :utc_datetime_usec, allow_nil?: false, public?: true
    attribute :lng, :float, allow_nil?: false, public?: true
    attribute :lat, :float, allow_nil?: false, public?: true
    attribute :speed_kph, :integer, allow_nil?: false, public?: true
    attribute :battery_pct, :integer, allow_nil?: false, public?: true
  end

  relationships do
    belongs_to :cab, Cybercab.Fleet.Cab, allow_nil?: false, public?: true
  end

  actions do
    defaults [:read]

    create :record do
      primary? true
      accept [:cab_id, :recorded_at, :lng, :lat, :speed_kph, :battery_pct]
    end

    destroy :prune do
      primary? true
    end
  end

  graphql do
    type :telemetry_sample

    queries do
      get :get_telemetry_sample, :read
      list :list_telemetry_samples, :read
    end

    mutations do
      create :record_telemetry_sample, :record
      destroy :prune_telemetry_sample, :prune
    end

    subscriptions do
      pubsub CybercabWeb.Endpoint
      subscribe(:telemetry_sample_created, do: action_types([:create]))
      subscribe(:telemetry_sample_updated, do: action_types([:update]))
      subscribe(:telemetry_sample_destroyed, do: action_types([:destroy]))
    end
  end
end
