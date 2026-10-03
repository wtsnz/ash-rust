defmodule Cybercab.Sim.Seed do
  @moduledoc """
  A lived-in Austin to start from, as the Rust Cybercab seeds it: the zones and hubs,
  regular riders, the fleet spread across town (some charging, a couple in the shop), the
  last two hours of trips, and a few minutes of pulse. It draws from the same random
  stream in the same order, so the same seed gives the same city.
  """

  require Ash.Query

  alias Cybercab.Fleet.{Cab, Depot}
  alias Cybercab.Riders.Rider
  alias Cybercab.Rides.{ServiceZone, Trip}
  alias Cybercab.Sim
  alias Cybercab.Sim.{City, Rng}
  alias Cybercab.Telemetry.PulseSample

  @first_names ~w(Priya Marcus Elena Jamal Sofia Diego Hannah Kenji Amara Luis Grace Omar
    Chloe Tariq Maya Wyatt Zoe Rafael Nina Caleb Ines Theo Ava Mateo Leah Ravi Isla Andre
    Freya Hugo Lena Owen Tessa Yusuf Mila Jonah Ruby Dante Esme Felix)
  @last_initials "ABCDEFGHJKLMNPRSTVWY"

  # What the night shift calls the cabs.
  @nicknames [
    "Bluebonnet",
    "Mesquite",
    "Pecan",
    "Grackle",
    "Armadillo",
    "Longhorn",
    "Live Oak",
    "Bat Bridge",
    "Barton",
    "Colorado",
    "Zilker",
    "Cedar",
    "Mopac",
    "Lady Bird",
    "Pedernales",
    "Brazos",
    "Lavaca",
    "Guadalupe",
    "San Jacinto",
    "Red River",
    "Rainey",
    "Congress",
    "Lamar",
    "Shoal",
    "Waller",
    "Bouldin",
    "Travis",
    "Hyde",
    "Tarrytown",
    "Oltorf",
    "Riverside",
    "Montopolis",
    "Pease",
    "Mayfield"
  ]

  @doc "The fleet the command center starts with."
  def fleet_size, do: 34

  defp ago(minutes),
    do: DateTime.add(DateTime.utc_now(), -round(minutes * 60_000), :millisecond)

  @doc """
  Austin with a fleet of `fleet` cabs. Riders, charging and grounded cabs, and the trip
  history scale with it, so a bigger fleet looks like a busier city.
  """
  def austin(city, seed, fleet) do
    if fleet < fleet_size(), do: raise(ArgumentError, "a fleet of at least #{fleet_size()} cabs")
    Rng.seed(seed)
    scale = fleet / fleet_size()
    charging = div(fleet * 5 + fleet_size() - 1, fleet_size())
    grounded = div(fleet * 2 + fleet_size() - 1, fleet_size())

    zones =
      Enum.map(city.zones, fn spec ->
        {lng, lat} = spec.center

        Sim.create!(ServiceZone, :chart, %{
          code: spec.code,
          name: spec.name,
          lng: lng,
          lat: lat,
          radius_m: spec.radius_m,
          base_demand: spec.base_demand,
          surge: 1.0,
          event_boost: 1.0
        })
      end)

    depots =
      Enum.map(city.depots, fn hub ->
        {lng, lat} = hub.at

        depot =
          Sim.create!(Depot, :open, %{
            code: hub.code,
            name: hub.name,
            lng: lng,
            lat: lat,
            stalls: hub.stalls || 8
          })

        {hub.code, depot}
      end)

    riders =
      for i <- 0..(div(96 * fleet, fleet_size()) - 1) do
        name = Enum.at(@first_names, rem(i, length(@first_names)))
        initial = String.at(@last_initials, Rng.below(String.length(@last_initials)))

        tier =
          case Rng.unit() do
            r when r < 0.06 -> :founder
            r when r < 0.32 -> :plus
            _ -> :standard
          end

        rating = round(Rng.between(4.3, 5.0) * 100) / 100
        phone = String.pad_leading(Integer.to_string(Rng.below(10_000)), 4, "0")
        assisted = Rng.chance(0.07)

        Sim.create!(Rider, :sign_up, %{
          display_name: "#{name} #{initial}.",
          tier: tier,
          rating: rating,
          phone_last4: phone,
          assisted_boarding: assisted
        })
      end

    cabs =
      for i <- 0..(fleet - 1) do
        nickname = Enum.at(@nicknames, rem(i, length(@nicknames)))

        {stop, depot} =
          if i < charging or i >= fleet - grounded do
            # Charging, or in the shop: at a hub.
            {code, depot} = Enum.at(depots, rem(i, length(depots)))
            {City.stop(city, code), depot}
          else
            place = Rng.pick(city.places)

            {_, depot} =
              Enum.min_by(depots, fn {code, _} ->
                City.metres_between(City.stop(city, code).at, place.at)
              end)

            {place, depot}
          end

        battery = if i < charging, do: 30 + Rng.below(30), else: 45 + Rng.below(54)
        {lng, lat} = stop.at
        lng = lng + Rng.between(-0.00025, 0.00025)
        lat = lat + Rng.between(-0.0002, 0.0002)
        heading = Rng.below(360)
        odometer = round(Rng.between(2_000.0, 31_000.0) * 10) / 10

        cab =
          Sim.create!(Cab, :commission, %{
            call_sign: "CC-" <> String.pad_leading(Integer.to_string(101 + i), 4, "0"),
            nickname: nickname,
            vin: "7SAYC" <> String.pad_leading(Integer.to_string(4_810_220 + i * 37), 12, "0"),
            software: if(rem(i, 7) == 0, do: "FSD v14.3.0 (beta ring)", else: "FSD v14.2.1"),
            depot_id: depot.id,
            lng: lng,
            lat: lat,
            heading_deg: heading,
            speed_kph: 0,
            battery_pct: battery,
            range_km: trunc(battery * 4.6),
            odometer_km: odometer,
            cabin_temp_c: 21.5
          })

        cond do
          i < charging -> cab |> Sim.update!(:recall) |> Sim.update!(:plug_in)
          i >= fleet - grounded -> Sim.update!(cab, :ground)
          true -> cab
        end
      end

    cabs = List.to_tuple(cabs)
    riders = List.to_tuple(riders)

    # The last two hours.
    weights = Enum.map(city.zones, & &1.base_demand)

    history =
      for _ <- 1..min(div(132 * fleet, fleet_size()), 4_000) do
        zone_index = Rng.weighted(weights)
        zone = Enum.at(zones, zone_index)
        pickup = Rng.pick(City.places_in(city, Enum.at(city.zones, zone_index).code))
        dropoff = Rng.pick(Enum.reject(city.places, &(&1.code == pickup.code)))
        route = City.route(city, pickup.code, dropoff.code)
        surge = if Rng.chance(0.2), do: 1.2 + Rng.below(5) / 10, else: 1.0
        {Rng.between(4.0, 125.0), zone, pickup, dropoff, route, surge}
      end
      |> Enum.sort_by(&elem(&1, 0), :desc)

    Enum.reduce(history, 48_210, fn {minutes_ago, zone, pickup, dropoff, route, surge}, code ->
      code = code + 1
      rider = Rng.pick(riders)
      cab = elem(cabs, charging + Rng.below(fleet - charging - grounded))
      {pickup_lng, pickup_lat} = pickup.at
      {dropoff_lng, dropoff_lat} = dropoff.at

      trip =
        Sim.create!(Trip, :request, %{
          code: "R-#{code}",
          rider_id: rider.id,
          zone_id: zone.id,
          pickup_name: pickup.name,
          pickup_lng: pickup_lng,
          pickup_lat: pickup_lat,
          dropoff_name: dropoff.name,
          dropoff_lng: dropoff_lng,
          dropoff_lat: dropoff_lat,
          ride_polyline: City.encode(route),
          distance_m: trunc(route.distance_m),
          duration_s: trunc(route.duration_s),
          surge: surge,
          fare_cents: Sim.fare_cents(route.distance_m, route.duration_s, surge),
          requested_at: ago(minutes_ago)
        })

      if Rng.chance(0.05) do
        Sim.update!(trip, :cancel, %{
          cancelled_at: ago(minutes_ago - 1.5),
          cancel_reason: "Rider cancelled"
        })
      else
        wait = Rng.between(1.5, 6.5)
        ride = route.duration_s / 60
        rating = if Rng.chance(0.8), do: 5, else: 4

        trip
        |> Sim.update!(:assign, %{
          cab_id: cab.id,
          assigned_at: ago(minutes_ago - 0.2),
          pickup_eta_at: ago(minutes_ago - wait)
        })
        |> Sim.update!(:arrive, %{arrived_at: ago(minutes_ago - wait)})
        |> Sim.update!(:board, %{
          picked_up_at: ago(minutes_ago - wait - 0.6),
          dropoff_eta_at: ago(max(minutes_ago - wait - 0.6 - ride, 0.5))
        })
        |> Sim.update!(:complete, %{
          completed_at: ago(max(minutes_ago - wait - 0.6 - ride, 0.5)),
          rating: rating
        })
      end

      code
    end)

    # A few minutes of pulse, so the sparklines start with a history.
    completed = Trip |> Ash.Query.filter(status == :completed) |> Ash.read!()
    revenue = completed |> Enum.map(& &1.fare_cents) |> Enum.sum()

    Enum.reduce(60..1//-1, 14.0, fn i, busy ->
      busy = (busy + Rng.between(-1.5, 1.5)) |> max(8.0) |> min(22.0)
      on_trip = round(busy * scale * 0.62)
      dispatched = round(busy * scale) - on_trip
      returning = Rng.below(2)
      waiting = trunc(Rng.below(4) * scale)
      avg_wait = 240 + Rng.below(120)
      avg_battery = 64 + Rng.below(6)

      Sim.create!(PulseSample, :record, %{
        recorded_at: ago(i * 5 / 60),
        available: fleet - (charging + grounded) - on_trip - dispatched,
        dispatched: dispatched,
        on_trip: on_trip,
        returning: returning,
        charging: charging,
        maintenance: grounded,
        waiting: waiting,
        completed_today: length(completed) - div(i, 12),
        revenue_cents_today: revenue - i * 180,
        avg_wait_s: avg_wait,
        utilization_pct: round(busy * 100 / (fleet_size() - 2.0)),
        avg_battery_pct: avg_battery
      })

      busy
    end)

    :ok
  end
end
