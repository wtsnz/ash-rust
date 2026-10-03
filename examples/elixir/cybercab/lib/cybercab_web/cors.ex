defmodule CybercabWeb.Cors do
  @moduledoc "Lets the command center's frontend, served from another origin, call the API."
  @behaviour Plug

  import Plug.Conn

  @impl true
  def init(opts), do: opts

  @impl true
  def call(conn, _opts) do
    conn =
      conn
      |> put_resp_header("access-control-allow-origin", "*")
      |> put_resp_header("access-control-allow-headers", "*")
      |> put_resp_header("access-control-allow-methods", "GET, POST, OPTIONS")

    if conn.method == "OPTIONS", do: conn |> send_resp(204, "") |> halt(), else: conn
  end
end
