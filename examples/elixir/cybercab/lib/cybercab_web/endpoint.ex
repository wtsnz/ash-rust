defmodule CybercabWeb.Endpoint do
  use Phoenix.Endpoint, otp_app: :cybercab
  use AshGraphql.Subscription.Endpoint

  # Subscriptions over `graphql-transport-ws`, at the same path as the Rust server's.
  socket "/graphql/ws", CybercabWeb.GraphqlSocket,
    websocket: [path: "", subprotocols: ["graphql-transport-ws"]],
    longpoll: false

  plug CybercabWeb.Cors

  plug Plug.Parsers,
    parsers: [:urlencoded, :multipart, :json, Absinthe.Plug.Parser],
    pass: ["*/*"],
    json_decoder: Jason

  plug CybercabWeb.Router
end
