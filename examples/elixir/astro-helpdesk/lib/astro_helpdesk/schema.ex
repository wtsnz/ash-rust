defmodule AstroHelpdesk.Schema do
  @moduledoc "The GraphQL schema AshGraphql builds for the desk."
  use Absinthe.Schema
  use AshGraphql, domains: [AstroHelpdesk.Desk]

  query do
  end

  mutation do
  end

  subscription do
  end
end
