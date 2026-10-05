defmodule AstroHelpdeskWeb.Endpoint do
  use Phoenix.Endpoint, otp_app: :astro_helpdesk
  use Absinthe.Phoenix.Endpoint

  socket("/graphql/ws", AstroHelpdeskWeb.GraphqlSocket,
    websocket: [path: "", subprotocols: ["graphql-transport-ws"]],
    longpoll: false
  )

  plug(AstroHelpdeskWeb.Cors)

  plug(Plug.Parsers,
    parsers: [:urlencoded, :multipart, :json, Absinthe.Plug.Parser],
    pass: ["*/*"],
    json_decoder: Jason
  )

  plug(AstroHelpdeskWeb.Router)
end
