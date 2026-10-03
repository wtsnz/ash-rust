defmodule Cybercab.Sim do
  @moduledoc """
  The fleet, simulated, as the Rust Cybercab simulates it. Riders hail cabs across
  Austin; a dispatcher assigns the nearest healthy cab; cabs drive real streets to the
  pickup, wait at the curb, carry the rider, and head back to a hub to charge when they
  run low. Now and then something happens that a person should look at.

  The simulation owns nothing. Every change goes through the domain's actions, so the
  control room sees it live, and operator commands reach it the same way: it reads the
  domain every tick and does what it says.

  It ticks once a second. A tick that runs over a second is followed straight away by the
  next, as a Tokio interval that delays missed ticks does.
  """

  use GenServer

  require Ash.Query
  require Logger

  alias Cybercab.Fleet.Cab
  alias Cybercab.Riders.Rider
  alias Cybercab.Rides.{ServiceZone, Trip}
  alias Cybercab.Sim.{City, Metrics, Rng}
  alias Cybercab.Telemetry.{FleetAlert, PulseSample, TelemetrySample}

  @active_trip_states [:requested, :assigned, :arrived, :riding]
  # Ticks a rider waits for a cab before giving up.
  @patience_ticks 150
  @low_battery_pct 25
  @charged_pct 92
  # Concurrent report updates, for `HEARTBEAT=concurrent`: the database pool's size.
  @report_concurrency 20

  # Running the domain's actions.

  @doc false
  def create!(resource, action, input) do
    resource |> Ash.Changeset.for_create(action, input) |> Ash.create!()
  end

  @doc false
  def update!(record, action, input \\ %{}) do
    record |> Ash.Changeset.for_update(action, input) |> Ash.update!()
  end

  @doc "$2.50 to start, $1.10 a kilometre and $0.25 a minute, times the surge; at least $6."
  def fare_cents(distance_m, duration_s, surge) do
    base = 250.0 + 110.0 * distance_m / 1000 + 25.0 * duration_s / 60
    max(round(base * surge), 600)
  end

  @doc """
  How many riders hail in a zone this tick, when `per_tick` hail there on average for the
  standard fleet and `roll` is uniform in `[0, 1)`. The standard fleet sees at most one a
  tick; a fleet `fleet_scale` times bigger sees proportionally more.
  """
  def hails(per_tick, fleet_scale, roll) do
    expected = if fleet_scale > 1.0, do: per_tick * fleet_scale, else: min(per_tick, 0.9)
    whole = Float.floor(expected)
    trunc(whole) + if(roll < expected - whole, do: 1, else: 0)
  end

  # The server.

  def start_link(config), do: GenServer.start_link(__MODULE__, config, name: __MODULE__)

  @impl true
  def init(config) do
    Rng.seed(config.seed)
    city = City.get()
    riders = Rider |> Ash.read!() |> Enum.map(&{&1.id, &1.assisted_boarding}) |> List.to_tuple()
    stops = city.places ++ city.depots

    motion =
      Cab
      |> Ash.read!()
      |> Map.new(fn cab ->
        stop = Enum.min_by(stops, &City.metres_between(&1.at, {cab.lng, cab.lat})).code
        parking = {Rng.between(-0.00025, 0.00025), Rng.between(-0.0002, 0.0002)}

        {cab.id,
         %{
           stop: stop,
           leg: nil,
           held_ticks: 0,
           boarding: nil,
           obstruction: nil,
           low_battery_alerted: false,
           ticks_since_report: 0,
           ticks_since_sample: 0,
           parking: parking
         }}
      end)

    state = %{
      city: city,
      config: config,
      motion: motion,
      plans: %{},
      riders: riders,
      tick: 0,
      fleet_scale: max(map_size(motion) / Cybercab.Sim.Seed.fleet_size(), 1.0),
      next_trip_code: 48_210 + Ash.count!(Trip),
      recent_waits: [],
      reports: [],
      samples: []
    }

    send(self(), :tick)
    {:ok, state}
  end

  @impl true
  def handle_info(:tick, state) do
    started = System.monotonic_time(:microsecond)

    {ok?, state} =
      try do
        {true, step(state)}
      rescue
        error ->
          Logger.error("simulation tick #{state.tick + 1}: #{Exception.message(error)}")
          {false, %{state | tick: state.tick + 1, reports: [], samples: []}}
      end

    elapsed_ms = (System.monotonic_time(:microsecond) - started) / 1000
    Metrics.record(elapsed_ms, ok?)
    Process.send_after(self(), :tick, max(0, 1000 - round(elapsed_ms)))
    {:noreply, state}
  end

  @doc "One second of the fleet."
  def step(state) do
    state = %{state | tick: state.tick + 1}
    cabs = Ash.read!(Cab)

    active =
      Trip
      |> Ash.Query.filter(status in ^@active_trip_states)
      |> Ash.Query.sort(requested_at: :asc)
      |> Ash.read!()

    state =
      state
      |> follow_operators(cabs, active)
      |> hail()
      |> dispatch(cabs, active)

    state =
      Enum.reduce(cabs, state, fn cab, state ->
        try do
          drive(state, cab)
        rescue
          error ->
            Logger.error("simulation: #{cab.call_sign} #{Exception.message(error)}")
            state
        end
      end)

    state = send_reports(state)
    if rem(state.tick, 5) == 0, do: measure_zones(active)
    state = if rem(state.tick, 5) == 0, do: take_pulse(state), else: state
    if rem(state.tick, 10) == 0, do: settle_alerts()
    if rem(state.tick, 30) == 0, do: prune()
    state
  end

  defp now, do: DateTime.utc_now()

  # Simulated seconds from now, in real time.
  defp after_simulated(state, simulated_s) do
    real_ms = trunc(simulated_s / state.config.speedup * 1000)
    DateTime.add(DateTime.utc_now(), real_ms, :millisecond)
  end

  # Does what operators asked: cabs recalled to a hub, grounded or released, and trips
  # cancelled out from under the cab fetching them.
  defp follow_operators(state, cabs, active) do
    active_ids = MapSet.new(active, & &1.id)

    motion =
      Enum.reduce(cabs, state.motion, fn cab, motion ->
        case Map.fetch(motion, cab.id) do
          :error ->
            motion

          {:ok, m} ->
            case cab.status do
              :maintenance ->
                Map.put(motion, cab.id, %{m | leg: nil, boarding: nil})

              :returning ->
                if match?(%{purpose: :hub}, m.leg) do
                  motion
                else
                  hub = City.nearest_depot(state.city, position_of(state.city, m)).code
                  leg = leg(City.route(state.city, m.stop, hub), hub, :hub, 1.0)
                  Map.put(motion, cab.id, %{m | leg: leg})
                end

              :dispatched ->
                # The trip it was fetching was cancelled.
                if cab.trip_id && MapSet.member?(active_ids, cab.trip_id) do
                  motion
                else
                  update!(cab, :stand_down, %{trip_id: nil})
                  Map.put(motion, cab.id, %{m | leg: nil})
                end

              _ ->
                motion
            end
        end
      end)

    # Plans for trips that are over.
    %{
      state
      | motion: motion,
        plans: Map.filter(state.plans, fn {id, _} -> MapSet.member?(active_ids, id) end)
    }
  end

  defp leg(route, to, purpose, pace),
    do: %{route: route, travelled: 0.0, to: to, purpose: purpose, pace: pace}

  # Riders hail cabs, more where demand is high and where an event is on.
  defp hail(state) do
    ServiceZone
    |> Ash.read!()
    |> Enum.reduce(state, fn zone, state ->
      boost = if zone.event_name, do: max(zone.event_boost, 1.0), else: 1.0
      # Requests a simulated minute, spread over this tick's simulated seconds.
      per_tick =
        zone.base_demand * 0.27 * boost * state.config.demand * state.config.speedup / 60

      requests = hails(per_tick, state.fleet_scale, Rng.unit())
      pickups = state.city |> City.places_in(zone.code) |> Enum.map(& &1.code)

      if requests == 0 or pickups == [] do
        state
      else
        Enum.reduce(1..requests, state, fn _, state ->
          pickup = Rng.pick(pickups)
          dropoff = pick_destination(state.city, pickup)
          request(state, zone, pickup, dropoff)
        end)
      end
    end)
  end

  defp pick_destination(city, from) do
    places = Enum.reject(city.places, &(&1.code == from))

    weights =
      Enum.map(places, fn place ->
        case place.code do
          "AUSA" -> 2.2
          code when code in ["CON6", "RAI", "SOCO", "DOMN"] -> 1.6
          code when code in ["COTA", "GIGA"] -> 0.4
          _ -> 1.0
        end
      end)

    Enum.at(places, Rng.weighted(weights)).code
  end

  defp request(state, zone, pickup, dropoff) do
    route = City.route(state.city, pickup, dropoff)
    {rider_id, assisted} = Rng.pick(state.riders)
    from = City.stop(state.city, pickup)
    to = City.stop(state.city, dropoff)
    code = state.next_trip_code + 1
    {from_lng, from_lat} = from.at
    {to_lng, to_lat} = to.at

    trip =
      create!(Trip, :request, %{
        code: "R-#{code}",
        rider_id: rider_id,
        zone_id: zone.id,
        pickup_name: from.name,
        pickup_lng: from_lng,
        pickup_lat: from_lat,
        dropoff_name: to.name,
        dropoff_lng: to_lng,
        dropoff_lat: to_lat,
        ride_polyline: City.encode(route),
        distance_m: trunc(route.distance_m),
        duration_s: trunc(route.duration_s),
        surge: zone.surge,
        fare_cents: fare_cents(route.distance_m, route.duration_s, zone.surge),
        requested_at: now()
      })

    plan = %{pickup: pickup, dropoff: dropoff, waited_ticks: 0, assisted: assisted}
    %{state | next_trip_code: code, plans: Map.put(state.plans, trip.id, plan)}
  end

  # Assigns each waiting rider the nearest healthy cab, oldest request first. A rider who
  # waits too long gives up.
  defp dispatch(state, cabs, active) do
    free =
      Enum.filter(cabs, fn cab ->
        cab.status == :available and not cab.halted and cab.battery_pct >= @low_battery_pct
      end)

    {state, _free} =
      active
      |> Enum.filter(&(&1.status == :requested))
      |> Enum.reduce({state, free}, fn trip, {state, free} ->
        case Map.fetch(state.plans, trip.id) do
          :error ->
            {state, free}

          {:ok, plan} ->
            plan = %{plan | waited_ticks: plan.waited_ticks + 1}
            state = %{state | plans: Map.put(state.plans, trip.id, plan)}
            pickup_at = City.stop(state.city, plan.pickup).at

            nearest =
              free
              |> Enum.filter(&Map.has_key?(state.motion, &1.id))
              |> Enum.min_by(
                &City.metres_between(position_of(state.city, state.motion[&1.id]), pickup_at),
                &<=/2,
                fn -> nil end
              )

            case nearest do
              nil ->
                if plan.waited_ticks > @patience_ticks do
                  update!(trip, :cancel, %{cancelled_at: now(), cancel_reason: "No cab nearby"})
                end

                {state, free}

              cab ->
                m = state.motion[cab.id]
                approach = City.route(state.city, m.stop, plan.pickup)

                update!(trip, :assign, %{
                  cab_id: cab.id,
                  approach_polyline: City.encode(approach),
                  assigned_at: now(),
                  pickup_eta_at: after_simulated(state, approach.duration_s)
                })

                update!(cab, :dispatch, %{trip_id: trip.id})
                pace = Rng.between(0.9, 1.1)

                m = %{
                  m
                  | boarding: nil,
                    leg: leg(approach, plan.pickup, {:pickup, trip.id}, pace)
                }

                {%{state | motion: Map.put(state.motion, cab.id, m)}, List.delete(free, cab)}
            end
        end
      end)

    state
  end

  # Moves one cab a tick along, and reports what it's doing.
  defp drive(state, cab) do
    case Map.fetch(state.motion, cab.id) do
      :error ->
        state

      {:ok, m} ->
        {m, state} = drive_with(state, cab, m)
        %{state | motion: Map.put(state.motion, cab.id, m)}
    end
  end

  defp drive_with(state, cab, m) do
    battery = cab.battery_pct
    odometer = cab.odometer_km
    heading = cab.heading_deg

    # At the curb, a rider boards.
    {m, state} =
      case m.boarding do
        {trip_id, ticks} when ticks > 0 -> {%{m | boarding: {trip_id, ticks - 1}}, state}
        {trip_id, _} -> board(state, cab, %{m | boarding: nil}, trip_id)
        nil -> {m, state}
      end

    {m, battery, odometer, speed_kph, heading, moved} =
      cond do
        cab.status == :charging ->
          battery = min(battery + 2, 100)

          if battery >= @charged_pct do
            update!(cab, :unplug)
            {%{m | low_battery_alerted: false}, battery, odometer, 0, heading, false}
          else
            {m, battery, odometer, 0, heading, false}
          end

        m.leg != nil and not cab.halted and cab.status != :maintenance ->
          if m.held_ticks > 0 do
            {%{m | held_ticks: m.held_ticks - 1}, battery, odometer, 0, heading, false}
          else
            if m.obstruction, do: auto_resolve(m.obstruction)
            m = %{m | obstruction: nil}
            leg = m.leg
            remaining = leg.route.distance_m - leg.travelled
            # Pulling away and pulling in are slower than cruising.
            ramp = (min(leg.travelled, remaining) / 180) |> max(0.35) |> min(1.0)
            mps = City.speed_mps(leg.route) * leg.pace * ramp * Rng.between(0.85, 1.1)
            step = min(mps * state.config.speedup, remaining)
            leg = %{leg | travelled: leg.travelled + step}
            odometer = odometer + step / 1000
            battery = max(round(battery - step * 0.00045 - Rng.between(0.0, 0.2)), 1)
            {_, bearing} = City.position(leg.route, leg.travelled)
            m = maybe_incident(state, cab, %{m | leg: leg})
            {m, battery, odometer, round(mps * 3.6), round(bearing), true}
          end

        true ->
          {m, battery, odometer, 0, heading, false}
      end

    # Arrived?
    arrived = m.leg != nil and m.leg.travelled >= m.leg.route.distance_m - 0.5

    {m, state, speed_kph} =
      if arrived do
        leg = m.leg
        m = %{m | leg: nil, stop: leg.to}

        {m, state} =
          case leg.purpose do
            {:pickup, trip_id} ->
              Trip |> Ash.get!(trip_id) |> update!(:arrive, %{arrived_at: now()})
              assisted = match?(%{assisted: true}, state.plans[trip_id])
              wait = Rng.below(4) + 3 + if(assisted, do: 5, else: 0)
              m = %{m | boarding: {trip_id, wait}}

              if Rng.chance(0.04) do
                raise_alert(
                  state,
                  cab,
                  m,
                  trip_id,
                  :door_ajar,
                  :info,
                  "Rear door reported ajar at pickup"
                )
              end

              {m, state}

            {:ride, trip_id} ->
              drop_off(state, cab, m, trip_id, battery)

            :hub ->
              update!(cab, :plug_in)
              {m, state}
          end

        {m, state, 0}
      else
        {m, state, speed_kph}
      end

    # Report: every tick on the move or when anything changed, now and then when parked.
    m = %{m | ticks_since_report: m.ticks_since_report + 1}
    changed = speed_kph != cab.speed_kph or battery != cab.battery_pct

    {m, state} =
      if moved or arrived or changed or m.ticks_since_report >= 10 do
        {lng, lat} = position_of(state.city, m)

        report = %{
          lng: lng,
          lat: lat,
          heading_deg: heading,
          speed_kph: speed_kph,
          battery_pct: battery,
          range_km: round(battery * 4.6),
          odometer_km: round(odometer * 10) / 10,
          cabin_temp_c: 21.5 + Rng.between(-0.6, 0.6),
          last_seen_at: now()
        }

        {%{m | ticks_since_report: 0}, %{state | reports: [{cab, report} | state.reports]}}
      else
        {m, state}
      end

    m = %{m | ticks_since_sample: m.ticks_since_sample + 1}

    {m, state} =
      if moved and m.ticks_since_sample >= 3 do
        {lng, lat} = position_of(state.city, m)

        sample = %{
          cab_id: cab.id,
          recorded_at: now(),
          lng: lng,
          lat: lat,
          speed_kph: speed_kph,
          battery_pct: battery
        }

        {%{m | ticks_since_sample: 0}, %{state | samples: [sample | state.samples]}}
      else
        {m, state}
      end

    m =
      if battery < 20 and not m.low_battery_alerted do
        raise_alert(state, cab, m, cab.trip_id, :low_battery, :warning, "Battery at #{battery}%")
        %{m | low_battery_alerted: true}
      else
        m
      end

    {m, state}
  end

  # Writes this tick's position reports and telemetry samples, the heartbeat as
  # `HEARTBEAT` says, and the samples with one bulk create.
  defp send_reports(state) do
    reports = Enum.reverse(state.reports)

    if reports != [] do
      heartbeat(state.config.heartbeat, reports)
    end

    samples = Enum.reverse(state.samples)

    if samples != [] do
      result =
        Ash.bulk_create(samples, TelemetrySample, :record,
          notify?: true,
          return_errors?: true,
          stop_on_error?: false
        )

      for error <- List.wrap(result.errors),
          do: Logger.error("simulation: sample #{inspect(error)}")
    end

    %{state | reports: [], samples: []}
  end

  # Each cab's report, as its own `report` update, run concurrently.
  defp heartbeat("concurrent", reports) do
    reports
    |> Task.async_stream(
      fn {cab, report} -> Ash.update(cab, report, action: :report) end,
      max_concurrency: @report_concurrency,
      ordered: false,
      timeout: :infinity
    )
    |> Enum.each(fn
      {:ok, {:ok, _}} -> :ok
      {:ok, {:error, error}} -> Logger.error("simulation: report #{inspect(error)}")
    end)
  end

  # Every cab's report in one `Ash.bulk_create` upsert.
  defp heartbeat("upsert", reports) do
    rows =
      Enum.map(reports, fn {cab, report} ->
        cab
        |> Map.take([:call_sign, :nickname, :vin, :software, :depot_id])
        |> Map.merge(report)
      end)

    result =
      Ash.bulk_create(rows, Cab, :heartbeat,
        notify?: true,
        return_errors?: true,
        stop_on_error?: false
      )

    for error <- List.wrap(result.errors),
        do: Logger.error("simulation: report #{inspect(error)}")
  end

  defp board(state, cab, m, trip_id) do
    trip = Ash.get!(Trip, trip_id)

    if trip.status != :arrived do
      {m, state}
    else
      picked_up = now()

      waited =
        trunc(
          DateTime.diff(picked_up, trip.requested_at, :millisecond) / 1000 * state.config.speedup
        )

      state = %{state | recent_waits: Enum.take(state.recent_waits ++ [waited], -30)}

      case state.plans[trip_id] do
        nil ->
          {m, state}

        plan ->
          route = City.route(state.city, plan.pickup, plan.dropoff)

          update!(trip, :board, %{
            picked_up_at: picked_up,
            dropoff_eta_at: after_simulated(state, route.duration_s)
          })

          Cab |> Ash.get!(cab.id) |> update!(:begin_ride)
          pace = Rng.between(0.92, 1.08)
          {%{m | leg: leg(route, plan.dropoff, {:ride, trip_id}, pace)}, state}
      end
    end
  end

  defp drop_off(state, cab, m, trip_id, battery) do
    rating =
      case Rng.unit() do
        r when r < 0.78 -> 5
        r when r < 0.95 -> 4
        r when r < 0.99 -> 3
        _ -> 2
      end

    Trip |> Ash.get!(trip_id) |> update!(:complete, %{completed_at: now(), rating: rating})
    cab = Cab |> Ash.get!(cab.id) |> update!(:finish_ride, %{trip_id: nil})

    if battery < @low_battery_pct do
      hub = City.nearest_depot(state.city, position_of(state.city, m)).code
      route = City.route(state.city, m.stop, hub)
      update!(cab, :recall)
      {%{m | leg: leg(route, hub, :hub, 1.0)}, state}
    else
      {m, state}
    end
  end

  # Now and then, something a person should look at.
  defp maybe_incident(state, cab, m) do
    riding = cab.status == :on_trip
    roll = Rng.unit()

    cond do
      roll < 0.0011 ->
        held = 6 + Rng.below(8)
        message = "Stopped for an obstruction in the lane"
        id = raise_alert(state, cab, m, cab.trip_id, :obstruction, :warning, message)
        %{m | held_ticks: held, obstruction: id}

      roll < 0.0019 ->
        raise_alert(
          state,
          cab,
          m,
          cab.trip_id,
          :hard_braking,
          :warning,
          "Hard braking event (0.6 g)"
        )

        m

      riding and roll < 0.0025 ->
        message = "Rider pressed the assist button"
        raise_alert(state, cab, m, cab.trip_id, :rider_assist, :critical, message)
        m

      roll < 0.0027 ->
        message = "Front-left camera degraded: glare"
        raise_alert(state, cab, m, cab.trip_id, :sensor_degraded, :info, message)
        m

      true ->
        m
    end
  end

  defp raise_alert(state, cab, m, trip_id, kind, severity, message) do
    {lng, lat} = position_of(state.city, m)

    alert =
      create!(FleetAlert, :raise, %{
        cab_id: cab.id,
        trip_id: trip_id,
        kind: kind,
        severity: severity,
        message: "#{cab.call_sign} · #{message}",
        lng: lng,
        lat: lat,
        raised_at: now()
      })

    alert.id
  end

  defp auto_resolve(alert_id) do
    case Ash.get(FleetAlert, alert_id) do
      {:ok, %{status: status} = alert} when status != :resolved ->
        update!(alert, :resolve, %{resolved_at: now(), handled_by: "auto"})

      _ ->
        :ok
    end
  end

  # Alerts nobody touched clear themselves after a few minutes, except rider assists.
  defp settle_alerts do
    stale = DateTime.add(DateTime.utc_now(), -3, :minute)

    FleetAlert
    |> Ash.Query.filter(status == :open and raised_at < ^stale and kind != :rider_assist)
    |> Ash.read!()
    |> Enum.each(&auto_resolve(&1.id))
  end

  # Riders waiting in each zone, and the surge they call for.
  defp measure_zones(active) do
    for zone <- Ash.read!(ServiceZone) do
      waiting = Enum.count(active, &(&1.zone_id == zone.id and &1.status == :requested))
      boost = if zone.event_name, do: zone.event_boost, else: 1.0

      surge =
        round(min(max(1.0 + waiting * 0.15 + (boost - 1.0) * 0.2, 1.0), 3.0) * 10) / 10

      if zone.waiting != waiting or abs(zone.surge - surge) > 2.220446049250313e-16 do
        update!(zone, :measure, %{waiting: waiting, surge: surge})
      end
    end
  end

  defp take_pulse(state) do
    cabs = Ash.read!(Cab)
    counts = Enum.frequencies_by(cabs, & &1.status)
    count = &Map.get(counts, &1, 0)
    in_service = length(cabs) - count.(:maintenance)
    busy = count.(:dispatched) + count.(:on_trip)
    completed = Trip |> Ash.Query.filter(status == :completed) |> Ash.read!()
    waiting = Trip |> Ash.Query.filter(status == :requested) |> Ash.count!()

    avg_wait =
      if state.recent_waits == [],
        do: 0,
        else: div(Enum.sum(state.recent_waits), length(state.recent_waits))

    create!(PulseSample, :record, %{
      recorded_at: now(),
      available: count.(:available),
      dispatched: count.(:dispatched),
      on_trip: count.(:on_trip),
      returning: count.(:returning),
      charging: count.(:charging),
      maintenance: count.(:maintenance),
      waiting: waiting,
      completed_today: length(completed),
      revenue_cents_today: completed |> Enum.map(& &1.fare_cents) |> Enum.sum(),
      avg_wait_s: avg_wait,
      utilization_pct: if(in_service > 0, do: div(busy * 100, in_service), else: 0),
      avg_battery_pct:
        if(cabs == [],
          do: 0,
          else: div(cabs |> Enum.map(& &1.battery_pct) |> Enum.sum(), length(cabs))
        )
    })

    state
  end

  # Keeps the history bounded: recent trails, pulse and finished trips.
  defp prune do
    doomed =
      TelemetrySample
      |> Ash.Query.sort(recorded_at: :desc)
      |> Ash.read!()
      |> Enum.reduce({%{}, []}, fn sample, {kept, doomed} ->
        seen = Map.get(kept, sample.cab_id, 0) + 1
        kept = Map.put(kept, sample.cab_id, seen)
        {kept, if(seen > 120, do: [sample | doomed], else: doomed)}
      end)
      |> elem(1)

    destroy(doomed, :prune)

    PulseSample
    |> Ash.Query.sort(recorded_at: :desc)
    |> Ash.Query.offset(240)
    |> Ash.read!()
    |> destroy(:prune)

    Trip
    |> Ash.Query.filter(status in [:completed, :cancelled])
    |> Ash.Query.sort(requested_at: :desc)
    |> Ash.Query.offset(400)
    |> Ash.read!()
    |> destroy(:archive)
  end

  defp destroy([], _action), do: :ok

  defp destroy(records, action) do
    Ash.bulk_destroy(records, action, %{},
      notify?: true,
      return_errors?: true,
      stop_on_error?: false
    )

    :ok
  end

  # Where a cab is: along its leg, or parked at its stop.
  defp position_of(city, m) do
    case m.leg do
      %{travelled: travelled, route: route} when travelled > 0 ->
        route |> City.position(travelled) |> elem(0)

      _ ->
        {lng, lat} = City.stop(city, m.stop).at
        {dlng, dlat} = m.parking
        {lng + dlng, lat + dlat}
    end
  end
end
