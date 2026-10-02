defmodule Cybercab.Telemetry.FleetAlert do
  @moduledoc "Something a person should look at: open, acknowledged, then resolved."
  use Ash.Resource,
    domain: Cybercab.Telemetry,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshStateMachine, AshGraphql.Resource],
    notifiers: [Ash.Notifier.PubSub]

  postgres do
    table "fleet_alerts"
    repo Cybercab.Repo

    custom_indexes do
      index [:raised_at], name: "fleet_alerts_open_by_raised", where: "status <> 'resolved'"
    end
  end

  state_machine do
    state_attribute :status
    initial_states [:open]
    default_initial_state :open

    transitions do
      transition :acknowledge, from: :open, to: :acknowledged
      transition :resolve, from: [:open, :acknowledged], to: :resolved
    end
  end

  attributes do
    uuid_primary_key :id
    attribute :kind, Cybercab.Types.AlertKind, allow_nil?: false, public?: true
    attribute :severity, Cybercab.Types.Severity, allow_nil?: false, public?: true
    attribute :message, :string, allow_nil?: false, public?: true
    attribute :lng, :float, allow_nil?: false, public?: true
    attribute :lat, :float, allow_nil?: false, public?: true
    attribute :raised_at, :utc_datetime_usec, allow_nil?: false, public?: true
    attribute :acknowledged_at, :utc_datetime_usec, public?: true
    attribute :resolved_at, :utc_datetime_usec, public?: true
    attribute :handled_by, :string, public?: true
  end

  relationships do
    belongs_to :cab, Cybercab.Fleet.Cab, allow_nil?: false, public?: true
    belongs_to :trip, Cybercab.Rides.Trip, public?: true
  end

  actions do
    defaults [:read]

    create :raise do
      primary? true
      accept [:cab_id, :trip_id, :kind, :severity, :message, :lng, :lat, :raised_at]
    end

    update :acknowledge do
      accept [:acknowledged_at, :handled_by]
      change transition_state(:acknowledged)
    end

    update :resolve do
      accept [:resolved_at, :handled_by]
      change transition_state(:resolved)
    end

    destroy :prune do
      primary? true
    end
  end

  graphql do
    type :fleet_alert

    queries do
      get :get_fleet_alert, :read
      list :list_fleet_alerts, :read
    end

    mutations do
      create :raise_fleet_alert, :raise
      update :acknowledge_fleet_alert, :acknowledge
      update :resolve_fleet_alert, :resolve
      destroy :prune_fleet_alert, :prune
    end

    subscriptions do
      pubsub CybercabWeb.Endpoint
      subscribe(:fleet_alert_created, do: action_types([:create]))
      subscribe(:fleet_alert_updated, do: action_types([:update]))
      subscribe(:fleet_alert_destroyed, do: action_types([:destroy]))
    end
  end
end
