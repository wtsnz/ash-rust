defmodule CybercabWeb.GraphqlSocket do
  use Absinthe.GraphqlWS.Socket, schema: Cybercab.Schema

  @impl true
  def handle_message(_message, socket), do: {:ok, socket}
end
