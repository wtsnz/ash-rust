import Config

config :cybercab, CybercabWeb.Endpoint, secret_key_base: String.duplicate("cybercab-dev-only-", 4)
