defmodule Cybercab.Telemetry.PulseSample do
  @moduledoc "The room's vital signs at a moment, for the sparklines."
  use Ash.Resource,
    domain: Cybercab.Telemetry,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshGraphql.Resource],
    notifiers: [Ash.Notifier.PubSub]

  postgres do
    table "pulse_samples"
    repo Cybercab.Repo

    custom_indexes do
      index [:recorded_at], name: "pulse_samples_by_time"
    end
  end

  attributes do
    uuid_primary_key :id
    attribute :recorded_at, :utc_datetime_usec, allow_nil?: false, public?: true

    for field <- [
          :available,
          :dispatched,
          :on_trip,
          :returning,
          :charging,
          :maintenance,
          :waiting,
          :completed_today,
          :revenue_cents_today,
          :avg_wait_s,
          :utilization_pct,
          :avg_battery_pct
        ] do
      attribute field, :integer, allow_nil?: false, public?: true
    end
  end

  actions do
    defaults [:read]

    create :record do
      primary? true

      accept [
        :recorded_at,
        :available,
        :dispatched,
        :on_trip,
        :returning,
        :charging,
        :maintenance,
        :waiting,
        :completed_today,
        :revenue_cents_today,
        :avg_wait_s,
        :utilization_pct,
        :avg_battery_pct
      ]
    end

    destroy :prune do
      primary? true
    end
  end

  graphql do
    type :pulse_sample

    queries do
      get :get_pulse_sample, :read
      list :list_pulse_samples, :read
    end

    mutations do
      create :record_pulse_sample, :record
      destroy :prune_pulse_sample, :prune
    end

    subscriptions do
      pubsub CybercabWeb.Endpoint
      subscribe(:pulse_sample_created, do: action_types([:create]))
      subscribe(:pulse_sample_updated, do: action_types([:update]))
      subscribe(:pulse_sample_destroyed, do: action_types([:destroy]))
    end
  end
end
