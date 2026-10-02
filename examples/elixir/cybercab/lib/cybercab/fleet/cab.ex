defmodule Cybercab.Fleet.Cab do
  @moduledoc """
  A Cybercab: two seats, no steering wheel, no pedals. It reports where it is, how fast
  it's going and how much charge it has every second it's on the road.

  Its status is the dispatch cycle: available, dispatched to a pickup, on a trip, then
  available again, with detours back to a hub to charge and out of service.
  """
  use Ash.Resource,
    domain: Cybercab.Fleet,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshStateMachine, AshGraphql.Resource],
    notifiers: [Ash.Notifier.PubSub]

  postgres do
    table "cabs"
    repo Cybercab.Repo

    check_constraints do
      check_constraint :battery_pct, "battery_range", check: "battery_pct BETWEEN 0 AND 100"
    end
  end

  state_machine do
    state_attribute :status
    initial_states [:available]
    default_initial_state :available

    transitions do
      transition :dispatch, from: :available, to: :dispatched
      transition :begin_ride, from: :dispatched, to: :on_trip
      transition :finish_ride, from: :on_trip, to: :available
      transition :stand_down, from: :dispatched, to: :available
      transition :recall, from: :available, to: :returning
      transition :plug_in, from: :returning, to: :charging
      transition :unplug, from: :charging, to: :available
      transition :ground, from: [:available, :returning, :charging], to: :maintenance
      transition :release, from: :maintenance, to: :available
    end
  end

  attributes do
    uuid_primary_key :id
    # Fleet call sign, as painted on the door: "CC-0142".
    attribute :call_sign, :string, allow_nil?: false, public?: true
    # What the night shift calls it.
    attribute :nickname, :string, allow_nil?: false, public?: true
    attribute :vin, :string, allow_nil?: false, public?: true
    attribute :software, :string, allow_nil?: false, public?: true
    attribute :lng, :float, allow_nil?: false, public?: true
    attribute :lat, :float, allow_nil?: false, public?: true
    attribute :heading_deg, :integer, allow_nil?: false, public?: true
    attribute :speed_kph, :integer, allow_nil?: false, public?: true
    attribute :battery_pct, :integer, allow_nil?: false, public?: true
    attribute :range_km, :integer, allow_nil?: false, public?: true
    attribute :odometer_km, :float, allow_nil?: false, public?: true
    attribute :cabin_temp_c, :float, allow_nil?: false, public?: true
    # Stopped at the curb by an operator, whatever it was doing.
    attribute :halted, :boolean, allow_nil?: false, default: false, public?: true
    attribute :last_seen_at, :utc_datetime_usec, public?: true
    create_timestamp :created_at, public?: true
    update_timestamp :updated_at, public?: true
  end

  identities do
    identity :unique_call_sign, [:call_sign]
    identity :unique_vin, [:vin]
  end

  relationships do
    belongs_to :depot, Cybercab.Fleet.Depot, allow_nil?: false, public?: true
    # The trip it's dispatched to or carrying.
    belongs_to :trip, Cybercab.Rides.Trip, public?: true
    has_many :trips, Cybercab.Rides.Trip, public?: true
  end

  aggregates do
    count :trips_completed, :trips do
      filter expr(status == :completed)
      public? true
    end

    sum :fares_cents, :trips, :fare_cents do
      filter expr(status == :completed)
      public? true
    end
  end

  actions do
    defaults [:read]

    create :commission do
      primary? true

      accept [
        :call_sign,
        :nickname,
        :vin,
        :software,
        :depot_id,
        :lng,
        :lat,
        :heading_deg,
        :speed_kph,
        :battery_pct,
        :range_km,
        :odometer_km,
        :cabin_temp_c
      ]
    end

    # The heartbeat: where the cab is and how it's doing.
    update :report do
      primary? true

      accept [
        :lng,
        :lat,
        :heading_deg,
        :speed_kph,
        :battery_pct,
        :range_km,
        :odometer_km,
        :cabin_temp_c,
        :last_seen_at
      ]
    end

    # The heartbeat for the whole fleet at once, for `HEARTBEAT=upsert`: an upsert on the
    # call sign that writes only what a report does. `Ash.bulk_create` runs a batch of
    # them as one `INSERT ... ON CONFLICT` statement. Being a create, it notifies as one.
    create :heartbeat do
      accept [
        :call_sign,
        :nickname,
        :vin,
        :software,
        :depot_id,
        :lng,
        :lat,
        :heading_deg,
        :speed_kph,
        :battery_pct,
        :range_km,
        :odometer_km,
        :cabin_temp_c,
        :last_seen_at
      ]

      upsert? true
      upsert_identity :unique_call_sign

      upsert_fields [
        :lng,
        :lat,
        :heading_deg,
        :speed_kph,
        :battery_pct,
        :range_km,
        :odometer_km,
        :cabin_temp_c,
        :last_seen_at,
        :updated_at
      ]
    end

    update :dispatch do
      accept [:trip_id]
      change transition_state(:dispatched)
    end

    update :begin_ride do
      change transition_state(:on_trip)
    end

    update :finish_ride do
      accept [:trip_id]
      change transition_state(:available)
    end

    update :stand_down do
      accept [:trip_id]
      change transition_state(:available)
    end

    # Sends an idle cab back to its hub to charge.
    update :recall do
      change transition_state(:returning)
    end

    update :plug_in do
      change transition_state(:charging)
    end

    update :unplug do
      change transition_state(:available)
    end

    # Takes the cab out of service.
    update :ground do
      change transition_state(:maintenance)
    end

    update :release do
      change transition_state(:available)
    end

    # Stops the cab at the curb wherever it is.
    update :pull_over do
      change set_attribute(:halted, true)
    end

    update :resume do
      change set_attribute(:halted, false)
    end
  end

  graphql do
    type :cab

    queries do
      get :get_cab, :read
      list :list_cabs, :read
    end

    mutations do
      create :commission_cab, :commission
      update :report_cab, :report
      update :dispatch_cab, :dispatch
      update :begin_ride_cab, :begin_ride
      update :finish_ride_cab, :finish_ride
      update :stand_down_cab, :stand_down
      update :recall_cab, :recall
      update :plug_in_cab, :plug_in
      update :unplug_cab, :unplug
      update :ground_cab, :ground
      update :release_cab, :release
      update :pull_over_cab, :pull_over
      update :resume_cab, :resume
    end

    subscriptions do
      pubsub CybercabWeb.Endpoint

      subscribe :cab_created do
        action_types [:create]
      end

      subscribe :cab_updated do
        action_types [:update]
      end

      subscribe :cab_destroyed do
        action_types [:destroy]
      end
    end
  end
end
