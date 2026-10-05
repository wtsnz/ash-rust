import Config

config :astro_helpdesk, AstroHelpdeskWeb.Endpoint,
  secret_key_base: String.duplicate("astro-helpdesk-test-only-", 3),
  server: false

# The tests seed their own data.
config :astro_helpdesk, seed?: false
