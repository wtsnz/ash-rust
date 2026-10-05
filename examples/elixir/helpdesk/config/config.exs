import Config

config :helpdesk,
  ecto_repos: [Helpdesk.Repo],
  ash_domains: [Helpdesk.Memory, Helpdesk.Sqlite]

# Count string length in codepoints, as SQL data layers do.
config :ash, default_string_length_count: :codepoints

config :logger, level: :warning

import_config "#{config_env()}.exs"
