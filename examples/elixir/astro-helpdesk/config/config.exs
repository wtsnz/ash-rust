import Config

config :astro_helpdesk, ash_domains: [AstroHelpdesk.Desk]

config :ash, :disable_async?, false

# Count string length in codepoints, as the Rust desk does.
config :ash, default_string_length_count: :codepoints

config :astro_helpdesk, AstroHelpdeskWeb.Endpoint,
  adapter: Bandit.PhoenixAdapter,
  pubsub_server: AstroHelpdesk.PubSub,
  server: false,
  render_errors: [formats: [json: AstroHelpdeskWeb.ErrorJSON]],
  # The Astro frontend runs on another origin.
  check_origin: false

config :logger, level: :warning

config :phoenix, :json_library, Jason

import_config "#{config_env()}.exs"
