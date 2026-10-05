import Config

config :astro_helpdesk, AstroHelpdeskWeb.Endpoint,
  secret_key_base: String.duplicate("astro-helpdesk-dev-only-", 3)
