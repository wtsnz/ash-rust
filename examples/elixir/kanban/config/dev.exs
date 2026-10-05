import Config

config :kanban, Kanban.Repo, database: Path.expand("../priv/kanban.db", __DIR__)
