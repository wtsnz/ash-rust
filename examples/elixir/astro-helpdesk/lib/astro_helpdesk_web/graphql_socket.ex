defmodule AstroHelpdeskWeb.GraphqlSocket do
  @moduledoc "Subscriptions over `graphql-transport-ws`."
  use Absinthe.GraphqlWS.Socket, schema: AstroHelpdesk.Schema

  @impl true
  def handle_message(_message, socket), do: {:ok, socket}
end
