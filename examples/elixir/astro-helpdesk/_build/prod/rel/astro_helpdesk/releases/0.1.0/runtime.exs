import Config

# The Rust server's settings, from the same variable.
port = String.to_integer(System.get_env("PORT", "4000"))

if config_env() != :test do
  config :astro_helpdesk, AstroHelpdeskWeb.Endpoint,
    http: [ip: {127, 0, 0, 1}, port: port],
    server: true,
    secret_key_base:
      System.get_env("SECRET_KEY_BASE", String.duplicate("astro-helpdesk-only-", 3))
end
