defmodule Cybercab.Rides.Trip do
  @moduledoc """
  One ride, from the request to the drop-off. Its three legs are the journey: the cab's
  approach to the pickup, the wait at the curb, and the ride itself. Both legs' routes
  travel with the trip as encoded polylines.
  """
  use Ash.Resource,
    domain: Cybercab.Rides,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshStateMachine, AshGraphql.Resource],
    notifiers: [Ash.Notifier.PubSub]

  postgres do
    table "trips"
    repo Cybercab.Repo

    custom_indexes do
      index [:requested_at], name: "trips_by_requested"

      index [:cab_id],
        name: "trips_active_by_cab",
        where: "status IN ('assigned', 'arrived', 'riding')"
    end
  end

  state_machine do
    state_attribute :status
    initial_states [:requested]
    default_initial_state :requested

    transitions do
      transition :assign, from: :requested, to: :assigned
      transition :arrive, from: :assigned, to: :arrived
      transition :board, from: :arrived, to: :riding
      transition :complete, from: :riding, to: :completed
      transition :cancel, from: [:requested, :assigned, :arrived], to: :cancelled
    end
  end

  attributes do
    uuid_primary_key :id
    # Shown to riders and support: "R-48213".
    attribute :code, :string, allow_nil?: false, public?: true
    attribute :pickup_name, :string, allow_nil?: false, public?: true
    attribute :pickup_lng, :float, allow_nil?: false, public?: true
    attribute :pickup_lat, :float, allow_nil?: false, public?: true
    attribute :dropoff_name, :string, allow_nil?: false, public?: true
    attribute :dropoff_lng, :float, allow_nil?: false, public?: true
    attribute :dropoff_lat, :float, allow_nil?: false, public?: true
    # The ride leg, pickup to drop-off.
    attribute :ride_polyline, :string, allow_nil?: false, public?: true
    # The approach leg, from wherever the cab was when it was assigned.
    attribute :approach_polyline, :string, public?: true
    attribute :distance_m, :integer, allow_nil?: false, public?: true
    attribute :duration_s, :integer, allow_nil?: false, public?: true
    attribute :surge, :float, allow_nil?: false, public?: true
    attribute :fare_cents, :integer, allow_nil?: false, public?: true
    attribute :requested_at, :utc_datetime_usec, allow_nil?: false, public?: true
    attribute :assigned_at, :utc_datetime_usec, public?: true
    attribute :pickup_eta_at, :utc_datetime_usec, public?: true
    attribute :arrived_at, :utc_datetime_usec, public?: true
    attribute :picked_up_at, :utc_datetime_usec, public?: true
    attribute :dropoff_eta_at, :utc_datetime_usec, public?: true
    attribute :completed_at, :utc_datetime_usec, public?: true
    attribute :cancelled_at, :utc_datetime_usec, public?: true
    attribute :cancel_reason, :string, public?: true
    # The rider's rating of the ride, out of five.
    attribute :rating, :integer, public?: true
    create_timestamp :created_at, public?: true
    update_timestamp :updated_at, public?: true
  end

  identities do
    identity :unique_code, [:code]
  end

  relationships do
    belongs_to :rider, Cybercab.Riders.Rider, allow_nil?: false, public?: true
    belongs_to :cab, Cybercab.Fleet.Cab, public?: true
    belongs_to :zone, Cybercab.Rides.ServiceZone, allow_nil?: false, public?: true
  end

  calculations do
    calculate :route_label, :string, expr(pickup_name <> " → " <> dropoff_name), public?: true
  end

  actions do
    defaults [:read]

    create :request do
      primary? true

      accept [
        :code,
        :rider_id,
        :zone_id,
        :pickup_name,
        :pickup_lng,
        :pickup_lat,
        :dropoff_name,
        :dropoff_lng,
        :dropoff_lat,
        :ride_polyline,
        :distance_m,
        :duration_s,
        :surge,
        :fare_cents,
        :requested_at
      ]

      validate numericality(:fare_cents, greater_than_or_equal_to: 0)
    end

    read :recent do
      prepare build(sort: [requested_at: :desc], limit: 50)
    end

    update :assign do
      accept [:cab_id, :approach_polyline, :assigned_at, :pickup_eta_at]
      change transition_state(:assigned)
    end

    update :arrive do
      accept [:arrived_at]
      change transition_state(:arrived)
    end

    update :board do
      accept [:picked_up_at, :dropoff_eta_at]
      change transition_state(:riding)
    end

    update :complete do
      accept [:completed_at, :rating]
      validate numericality(:rating, greater_than_or_equal_to: 1, less_than_or_equal_to: 5)
      change transition_state(:completed)
    end

    # Called off: by the rider, or by an operator.
    update :cancel do
      accept [:cancelled_at, :cancel_reason]
      change transition_state(:cancelled)
    end

    destroy :archive do
      primary? true
    end
  end

  graphql do
    type :trip

    queries do
      get :get_trip, :read
      list :list_trips, :read
      list :recent_trips, :recent
    end

    mutations do
      create :request_trip, :request
      update :assign_trip, :assign
      update :arrive_trip, :arrive
      update :board_trip, :board
      update :complete_trip, :complete
      update :cancel_trip, :cancel
      destroy :archive_trip, :archive
    end

    subscriptions do
      pubsub CybercabWeb.Endpoint
      subscribe(:trip_created, do: action_types([:create]))
      subscribe(:trip_updated, do: action_types([:update]))
      subscribe(:trip_destroyed, do: action_types([:destroy]))
    end
  end
end
