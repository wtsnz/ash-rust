defmodule SupportdeskWeb.GraphqlPlug do
  @moduledoc "AshGraphql's schema, run as the request's actor and tenant."
  use Plug.Builder

  plug AshGraphql.Plug
  plug Absinthe.Plug, schema: Supportdesk.Schema
end
