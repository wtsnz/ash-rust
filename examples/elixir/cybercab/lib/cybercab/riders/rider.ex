defmodule Cybercab.Riders.Rider do
  @moduledoc "Someone who rides: their tier, their rating, and what they need at the curb."
  use Ash.Resource,
    domain: Cybercab.Riders,
    data_layer: AshPostgres.DataLayer,
    extensions: [AshGraphql.Resource],
    notifiers: [Ash.Notifier.PubSub]

  postgres do
    table "riders"
    repo Cybercab.Repo
  end

  attributes do
    uuid_primary_key :id
    attribute :display_name, :string, allow_nil?: false, public?: true
    attribute :tier, Cybercab.Types.RiderTier, allow_nil?: false, public?: true
    attribute :rating, :float, allow_nil?: false, public?: true
    attribute :phone_last4, :string, allow_nil?: false, public?: true
    attribute :assisted_boarding, :boolean, allow_nil?: false, default: false, public?: true
    create_timestamp :created_at, public?: true
    update_timestamp :updated_at, public?: true
  end

  relationships do
    has_many :trips, Cybercab.Rides.Trip, public?: true
  end

  aggregates do
    count :trip_count, :trips do
      filter expr(status == :completed)
      public? true
    end

    sum :lifetime_cents, :trips, :fare_cents do
      filter expr(status == :completed)
      public? true
    end
  end

  actions do
    defaults [:read]

    create :sign_up do
      primary? true
      accept [:display_name, :tier, :rating, :phone_last4, :assisted_boarding]
      validate numericality(:rating, greater_than_or_equal_to: 1, less_than_or_equal_to: 5)
    end
  end

  graphql do
    type :rider

    queries do
      get :get_rider, :read
      list :list_riders, :read
    end

    mutations do
      create :sign_up_rider, :sign_up
    end

    subscriptions do
      pubsub CybercabWeb.Endpoint
      subscribe(:rider_created, do: action_types([:create]))
      subscribe(:rider_updated, do: action_types([:update]))
      subscribe(:rider_destroyed, do: action_types([:destroy]))
    end
  end
end
