defmodule AstroHelpdeskWeb.Router do
  @moduledoc "The Rust server's paths: GraphQL at `/graphql`, GraphiQL at `/graphiql`, `/health`."
  use Plug.Router

  plug(:match)
  plug(:dispatch)

  get "/health", do: send_resp(conn, 200, "OK")

  forward("/graphiql",
    to: Absinthe.Plug.GraphiQL,
    init_opts: [
      schema: AstroHelpdesk.Schema,
      socket: AstroHelpdeskWeb.GraphqlSocket,
      interface: :playground
    ]
  )

  forward("/graphql", to: Absinthe.Plug, init_opts: [schema: AstroHelpdesk.Schema])

  match(_, do: send_resp(conn, 404, "Not found"))
end
