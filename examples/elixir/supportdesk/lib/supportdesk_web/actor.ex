defmodule SupportdeskWeb.Actor do
  @moduledoc """
  The tenant and actor a request runs as: the org `x-org` names, and the agent `x-actor`
  and `x-role` describe. The Rust desk reads the same headers.
  """
  @behaviour Plug

  @impl true
  def init(opts), do: opts

  @impl true
  def call(conn, _opts) do
    {tenant, actor} = from_headers(conn.req_headers)

    conn
    |> Ash.PlugHelpers.set_tenant(tenant)
    |> Ash.PlugHelpers.set_actor(actor)
  end

  @doc "The tenant and actor `headers` name, as `{tenant, actor}`."
  def from_headers(headers) do
    header = fn name -> Enum.find_value(headers, fn {k, v} -> if k == name, do: v end) end

    actor =
      case {header.("x-actor"), header.("x-role")} do
        {id, role} when is_binary(id) and is_binary(role) -> %{id: id, role: role}
        _ -> nil
      end

    {header.("x-org"), actor}
  end
end
