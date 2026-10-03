import Config

config :cybercab,
  ecto_repos: [Cybercab.Repo],
  ash_domains: [Cybercab.Fleet, Cybercab.Riders, Cybercab.Rides, Cybercab.Telemetry]

config :ash, :disable_async?, false

# Count string length in codepoints, as SQL data layers do.
config :ash, default_string_length_count: :codepoints

config :cybercab, Cybercab.Repo,
  # As many connections as the Rust server's pool.
  pool_size: 20

config :cybercab, CybercabWeb.Endpoint,
  adapter: Bandit.PhoenixAdapter,
  pubsub_server: Cybercab.PubSub,
  server: true,
  render_errors: [formats: [json: CybercabWeb.ErrorJSON]],
  # The command center's frontend runs on another origin.
  check_origin: false

config :logger, level: :warning

config :phoenix, :json_library, Jason

import_config "#{config_env()}.exs"
