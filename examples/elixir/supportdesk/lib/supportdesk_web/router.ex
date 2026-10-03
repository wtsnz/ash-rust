defmodule SupportdeskWeb.Router do
  @moduledoc """
  The desk's API, at the Rust desk's paths: GraphQL at `/graphql`, the JSON endpoints under
  `/api`, and `/health`. Every request runs as the tenant and actor its headers name.
  """
  use Plug.Router

  alias Supportdesk.Desk.Ticket

  plug :match
  plug SupportdeskWeb.Actor
  plug :dispatch

  get "/health" do
    if Supportdesk.Boot.ready?(),
      do: send_resp(conn, 200, "OK"),
      else: send_resp(conn, 503, "Seeding")
  end

  get "/health/seeded" do
    json(conn, 200, %{seeded_ms: Supportdesk.Boot.seeded_ms()})
  end

  # The `route` generic action. Returns the routed ticket's id.
  post "/api/route" do
    p = conn.body_params

    args = %{
      subject: p["subject"],
      body: p["body"],
      priority: p["priority"],
      requester_email: p["requesterEmail"],
      comments: p["comments"] || []
    }

    Ticket
    |> Ash.ActionInput.for_action(:route, args, opts(conn))
    |> Ash.run_action()
    |> respond(conn, &%{id: &1})
  end

  # Creates `count` tickets, assigns them all, then destroys them, each with a bulk action.
  # Bulk actions don't notify, by Ash's default.
  post "/api/bulk" do
    p = conn.body_params
    count = p["count"]
    opts = opts(conn)

    inputs =
      for i <- 0..(count - 1)//1 do
        %{
          subject: "Bulk import #{i}",
          body: "Imported from the old desk",
          priority: 1 + rem(i, 4),
          requester_email: "import#{i}@example.com",
          comments: []
        }
      end

    bulk = [return_errors?: true, stop_on_error?: true, notify?: false]
    # Ash picks the first strategy that can run: the lock version every update moves needs
    # each record's version, so updates stream record by record, as the Rust desk's do.
    strategy = [strategy: [:atomic, :atomic_batches, :stream]]

    with %Ash.BulkResult{status: :success, records: tickets} <-
           Ash.bulk_create(inputs, Ticket, :open, opts ++ bulk ++ [return_records?: true]),
         %Ash.BulkResult{status: :success} <-
           Ash.bulk_update(tickets, :assign, %{assignee_id: p["assigneeId"]}, opts ++ bulk ++ strategy),
         %Ash.BulkResult{status: :success} <-
           Ash.bulk_destroy(tickets, :destroy, %{}, opts ++ bulk ++ strategy) do
      n = length(tickets)
      json(conn, 200, %{created: n, updated: n, destroyed: n})
    else
      %Ash.BulkResult{errors: errors} -> failure(conn, List.first(errors || []))
    end
  end

  # Edits a ticket as of the version the client read, failing as stale if it has changed
  # since. Returns the new version.
  post "/api/edit" do
    p = conn.body_params
    opts = opts(conn)
    changes = Map.new(Map.take(p, ["subject", "priority"]), fn {k, v} -> {String.to_existing_atom(k), v} end)

    with {:ok, ticket} <- Ash.get(Ticket, p["id"], opts) do
      %{ticket | version: p["version"]}
      |> Ash.Changeset.for_update(:edit, changes, opts)
      |> Ash.update()
    end
    |> respond(conn, &%{version: &1.version})
  end

  # AshTypescript's RPC: an action by name, as the request's actor and tenant.
  post "/rpc/run" do
    json(conn, 200, AshTypescript.Rpc.run_action(:supportdesk, conn, conn.body_params))
  end

  post "/rpc/validate" do
    json(conn, 200, AshTypescript.Rpc.validate_action(:supportdesk, conn, conn.body_params))
  end

  forward "/graphql",
    to: SupportdeskWeb.GraphqlPlug

  match _ do
    send_resp(conn, 404, "Not found")
  end

  defp opts(conn) do
    [actor: Ash.PlugHelpers.get_actor(conn), tenant: Ash.PlugHelpers.get_tenant(conn)]
  end

  defp respond({:ok, value}, conn, body), do: json(conn, 200, body.(value))
  defp respond({:error, error}, conn, _body), do: failure(conn, error)

  # An error as JSON, with the status it means, as the Rust desk reports them.
  defp failure(conn, error) do
    error = Ash.Error.to_error_class(error)

    {status, code} =
      cond do
        match?(%Ash.Error.Forbidden{}, error) -> {403, "forbidden"}
        stale?(error) -> {409, "stale_record"}
        not_found?(error) -> {404, "not_found"}
        match?(%Ash.Error.Invalid{}, error) -> {422, "invalid"}
        true -> {500, "error"}
      end

    json(conn, status, %{error: code, message: Exception.message(error)})
  end

  defp stale?(error), do: Enum.any?(error.errors, &match?(%Ash.Error.Changes.StaleRecord{}, &1))

  defp not_found?(error), do: Enum.any?(error.errors, &match?(%Ash.Error.Query.NotFound{}, &1))

  defp json(conn, status, body) do
    conn |> put_resp_content_type("application/json") |> send_resp(status, Jason.encode!(body))
  end
end
