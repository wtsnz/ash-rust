import Config

config :cybercab, CybercabWeb.Endpoint,
  secret_key_base: String.duplicate("cybercab-test-only-", 4),
  server: false
