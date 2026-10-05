import Config

config :helpdesk, Helpdesk.Repo, database: Path.expand("../priv/helpdesk.db", __DIR__)
