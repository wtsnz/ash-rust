import Config

config :supportdesk,
  ecto_repos: [Supportdesk.Repo],
  ash_domains: [Supportdesk.Desk]

config :ash, :disable_async?, false

# AshTypescript's RPC: `/rpc/run` runs an action by name; its TypeScript client is generated
# where the benchmark's client reads it.
config :ash_typescript,
  manifest: Supportdesk.AshTypescriptManifest,
  output_file: "../../supportdesk/client/ash_rpc.ts",
  run_endpoint: "/rpc/run",
  validate_endpoint: "/rpc/validate",
  input_field_formatter: :camel_case,
  output_field_formatter: :camel_case

# Count string length in codepoints, as SQL data layers do.
config :ash, default_string_length_count: :codepoints

config :supportdesk, Supportdesk.Repo,
  # As many connections as the Rust server's pool.
  pool_size: 20

config :supportdesk, SupportdeskWeb.Endpoint,
  adapter: Bandit.PhoenixAdapter,
  pubsub_server: Supportdesk.PubSub,
  server: true,
  render_errors: [formats: [json: SupportdeskWeb.ErrorJSON]],
  check_origin: false

config :logger, level: :warning

config :phoenix, :json_library, Jason

import_config "#{config_env()}.exs"
