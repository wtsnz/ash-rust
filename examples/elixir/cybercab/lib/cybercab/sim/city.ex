defmodule Cybercab.Sim.City do
  @moduledoc """
  Austin as the simulation sees it: the places riders go, the Supercharger hubs, the
  service zones, and real driving routes between every pair of stops. The same city as
  the Rust Cybercab's, read from `examples/shared/cybercab`.

  Loaded once, into `:persistent_term`, and read from there.
  """

  import Bitwise

  @shared Path.expand("../../../../../shared/cybercab", __DIR__)
  @external_resource Path.join(@shared, "austin.json")
  @external_resource Path.join(@shared, "routes.json")
  @austin File.read!(Path.join(@shared, "austin.json"))
  @routes File.read!(Path.join(@shared, "routes.json"))

  defmodule Route do
    @moduledoc "A drivable path between two stops, with the distance along it at every vertex."
    defstruct [:points, :cumulative, :distance_m, :duration_s]
  end

  @doc "The city, loading it on first use."
  def get do
    case :persistent_term.get(__MODULE__, nil) do
      nil ->
        city = load()
        :persistent_term.put(__MODULE__, city)
        city

      city ->
        city
    end
  end

  defp load do
    file = Jason.decode!(@austin)

    stop = fn spec ->
      %{
        code: spec["code"],
        name: spec["name"],
        zone: spec["zone"],
        at: point(spec["at"]),
        stalls: spec["stalls"]
      }
    end

    places = Enum.map(file["places"], stop)
    depots = Enum.map(file["depots"], stop)

    routes =
      for {key, baked} <- Jason.decode!(@routes)["routes"], reduce: %{} do
        routes ->
          [from, to] = String.split(key, "|")
          forward = route(decode_polyline(baked["polyline"]), baked["duration_s"] * 1.0)
          backward = route(Enum.reverse(Tuple.to_list(forward.points)), forward.duration_s)
          routes |> Map.put({to, from}, backward) |> Map.put({from, to}, forward)
      end

    %{
      center: point(file["center"]),
      zones:
        Enum.map(file["zones"], fn zone ->
          %{
            code: zone["code"],
            name: zone["name"],
            center: point(zone["center"]),
            radius_m: zone["radius_m"],
            base_demand: zone["base_demand"] * 1.0
          }
        end),
      places: places,
      depots: depots,
      stops: Map.new(places ++ depots, &{&1.code, &1}),
      routes: routes
    }
  end

  defp point([lng, lat]), do: {lng * 1.0, lat * 1.0}

  defp route(points, duration_s) do
    {cumulative, total} =
      points
      |> Enum.chunk_every(2, 1, :discard)
      |> Enum.map_reduce(0.0, fn [a, b], total ->
        total = total + metres_between(a, b)
        {total, total}
      end)

    %Route{
      points: List.to_tuple(points),
      cumulative: List.to_tuple([0.0 | cumulative]),
      distance_m: total,
      duration_s: duration_s
    }
  end

  @doc "Staying put, for a cab already where it's going."
  def stationary(at), do: route([at, at], 0.0)

  def stop(city, code), do: Map.fetch!(city.stops, code)

  @doc "The driving route between two stops."
  def route(city, from, to) when from == to, do: stationary(stop(city, from).at)
  def route(city, from, to), do: Map.fetch!(city.routes, {from, to})

  def places_in(city, zone), do: Enum.filter(city.places, &(&1.zone == zone))

  @doc "The hub nearest `at`."
  def nearest_depot(city, at), do: Enum.min_by(city.depots, &metres_between(&1.at, at))

  @doc "Average driving speed along the route, in metres a second."
  def speed_mps(%Route{duration_s: duration_s, distance_m: distance_m}) when duration_s > 0,
    do: (distance_m / duration_s) |> max(4.0) |> min(30.0)

  def speed_mps(_route), do: 10.0

  @doc "Where a cab `travelled` metres along the route is, and which way it faces."
  def position(%Route{} = route, travelled) do
    travelled = travelled |> max(0.0) |> min(route.distance_m)
    count = tuple_size(route.points)
    segment = segment(route.cumulative, travelled, 0, count - 1) || max(count - 2, 0)
    a = elem(route.points, segment)
    b = elem(route.points, min(segment + 1, count - 1))

    length =
      if segment + 1 < tuple_size(route.cumulative),
        do: elem(route.cumulative, segment + 1) - elem(route.cumulative, segment),
        else: 0.0

    t = if length > 0, do: (travelled - elem(route.cumulative, segment)) / length, else: 1.0
    {{lng_a, lat_a}, {lng_b, lat_b}} = {a, b}
    {{lng_a + (lng_b - lng_a) * t, lat_a + (lat_b - lat_a) * t}, bearing(a, b)}
  end

  # The first segment whose far end is at least `travelled` along.
  defp segment(_cumulative, _travelled, i, last) when i >= last, do: nil

  defp segment(cumulative, travelled, i, last) do
    if travelled <= elem(cumulative, i + 1),
      do: i,
      else: segment(cumulative, travelled, i + 1, last)
  end

  def encode(%Route{points: points}), do: encode_polyline(Tuple.to_list(points))

  @doc "Great-circle distance in metres."
  def metres_between({lng_a, lat_a}, {lng_b, lat_b}) do
    lat1 = radians(lat_a)
    lat2 = radians(lat_b)
    dlat = lat2 - lat1
    dlng = radians(lng_b - lng_a)

    h =
      :math.pow(:math.sin(dlat / 2), 2) +
        :math.cos(lat1) * :math.cos(lat2) * :math.pow(:math.sin(dlng / 2), 2)

    6_371_000.0 * 2.0 * :math.asin(:math.sqrt(h))
  end

  @doc "Compass bearing from `a` to `b`, in degrees."
  def bearing({lng_a, lat_a}, {lng_b, lat_b}) do
    lat1 = radians(lat_a)
    lat2 = radians(lat_b)
    dlng = radians(lng_b - lng_a)
    y = :math.sin(dlng) * :math.cos(lat2)
    x = :math.cos(lat1) * :math.sin(lat2) - :math.sin(lat1) * :math.cos(lat2) * :math.cos(dlng)
    :math.fmod(:math.atan2(y, x) * 180 / :math.pi() + 360.0, 360.0)
  end

  defp radians(degrees), do: degrees * :math.pi() / 180

  @doc "Decodes a precision-5 encoded polyline into `{lng, lat}` points."
  def decode_polyline(encoded), do: decode(encoded, 0, 0, [])

  defp decode(<<>>, _lat, _lng, points), do: Enum.reverse(points)

  defp decode(encoded, lat, lng, points) do
    {dlat, rest} = decode_value(encoded, 0, 0)
    {dlng, rest} = decode_value(rest, 0, 0)
    lat = lat + dlat
    lng = lng + dlng
    decode(rest, lat, lng, [{lng / 1.0e5, lat / 1.0e5} | points])
  end

  defp decode_value(<<byte, rest::binary>>, shift, result) do
    chunk = byte - 63
    result = result ||| (chunk &&& 0x1F) <<< shift

    if chunk < 0x20 do
      {if((result &&& 1) == 1, do: bnot(result >>> 1), else: result >>> 1), rest}
    else
      decode_value(rest, shift + 5, result)
    end
  end

  @doc "Encodes `{lng, lat}` points as a precision-5 polyline."
  def encode_polyline(points) do
    {iodata, _, _} =
      Enum.reduce(points, {[], 0, 0}, fn {lng, lat}, {out, last_lat, last_lng} ->
        lat = round(lat * 1.0e5)
        lng = round(lng * 1.0e5)
        {[out, encode_value(lat - last_lat), encode_value(lng - last_lng)], lat, lng}
      end)

    IO.iodata_to_binary(iodata)
  end

  defp encode_value(value) do
    v = if value < 0, do: bnot(value <<< 1), else: value <<< 1
    encode_chunks(v, [])
  end

  defp encode_chunks(v, out) when v >= 0x20,
    do: encode_chunks(v >>> 5, [out, (0x20 ||| (v &&& 0x1F)) + 63])

  defp encode_chunks(v, out), do: [out, v + 63]
end
