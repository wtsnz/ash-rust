import Config

# One connection to one in-memory database, so every test sees the same one.
config :helpdesk, Helpdesk.Repo, database: ":memory:", pool_size: 1

config :ash, :disable_async?, true
