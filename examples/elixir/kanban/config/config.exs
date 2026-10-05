import Config

config :kanban,
  ecto_repos: [Kanban.Repo],
  ash_domains: [
    Kanban.Memory.Workspaces,
    Kanban.Memory.Boards,
    Kanban.Sqlite.Workspaces,
    Kanban.Sqlite.Boards
  ]

# Count string length in codepoints, as SQL data layers do.
config :ash, default_string_length_count: :codepoints

config :logger, level: :warning

import_config "#{config_env()}.exs"
