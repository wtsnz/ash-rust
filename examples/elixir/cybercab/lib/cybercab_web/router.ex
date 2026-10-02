defmodule CybercabWeb.Router do
  @moduledoc """
  The API, at the Rust server's paths: GraphQL at `/graphql`, GraphiQL at `/graphiql`,
  `/health`, and `/metrics` for the benchmark.
  """
  use Plug.Router

  plug :match
  plug :dispatch

  get "/health" do
    if Cybercab.Boot.ready?(),
      do: send_resp(conn, 200, "OK"),
      else: send_resp(conn, 503, "Seeding")
  end

  get "/metrics" do
    sim = Application.fetch_env!(:cybercab, :simulation)

    body =
      Cybercab.Sim.Metrics.to_map()
      |> Map.put(:fleet, sim[:fleet])
      |> Map.put(:simulating, sim[:enabled?])

    conn |> put_resp_content_type("application/json") |> send_resp(200, Jason.encode!(body))
  end

  forward "/graphiql",
    to: Absinthe.Plug.GraphiQL,
    init_opts: [
      schema: Cybercab.Schema,
      socket: CybercabWeb.GraphqlSocket,
      interface: :playground
    ]

  forward "/graphql", to: Absinthe.Plug, init_opts: [schema: Cybercab.Schema]

  match _ do
    send_resp(conn, 404, "Not found")
  end
end
