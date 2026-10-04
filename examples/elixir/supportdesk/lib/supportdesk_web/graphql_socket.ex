defmodule SupportdeskWeb.GraphqlSocket do
  use Absinthe.GraphqlWS.Socket, schema: Supportdesk.Schema

  import Absinthe.GraphqlWS.Util, only: [assign_context: 2]

  @impl true
  def handle_init(payload, socket) do
    {tenant, actor} = SupportdeskWeb.Actor.from_headers(socket.connect_info[:x_headers] || [])
    {:ok, payload, assign_context(socket, actor: actor, tenant: tenant)}
  end

  @impl true
  def handle_message(_message, socket), do: {:ok, socket}
end
