defmodule SupportdeskWeb.Endpoint do
  use Phoenix.Endpoint, otp_app: :supportdesk
  use AshGraphql.Subscription.Endpoint

  # Subscriptions over `graphql-transport-ws`, at the Rust desk's path. Each socket runs as
  # the `x-*` headers of its upgrade request say, as the Rust desk's do.
  socket "/graphql/ws", SupportdeskWeb.GraphqlSocket,
    websocket: [path: "", subprotocols: ["graphql-transport-ws"], connect_info: [:x_headers]],
    longpoll: false

  plug Plug.Parsers,
    parsers: [:urlencoded, :multipart, :json, Absinthe.Plug.Parser],
    pass: ["*/*"],
    json_decoder: Jason

  plug SupportdeskWeb.Router
end
