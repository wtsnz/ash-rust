import Config

# The same settings as the Rust Cybercab, from the same environment variables.
env = fn name, default -> System.get_env(name, default) end
number = fn name, default -> name |> env.(default) |> Float.parse() |> elem(0) end

config :cybercab, Cybercab.Repo,
  url: env.("DATABASE_URL", "postgres://postgres:postgres@127.0.0.1:5432/cybercab_elixir")

config :cybercab, CybercabWeb.Endpoint,
  http: [ip: {127, 0, 0, 1}, port: String.to_integer(env.("PORT", "4000"))]

config :cybercab, :simulation,
  enabled?: env.("SIM", "on") != "off",
  fleet: String.to_integer(env.("FLEET", "34")),
  speedup: number.("SIM_SPEED", "8"),
  demand: number.("DEMAND", "1"),
  seed: String.to_integer(env.("SEED", "51893")),
  # How the fleet's position reports are written: `concurrent` updates each cab with its
  # own `report` (an update, as in Rust), or `upsert` writes them all with one
  # `Ash.bulk_create` upsert, which AshPostgres runs as one statement.
  heartbeat: env.("HEARTBEAT", "concurrent")

if config_env() == :prod do
  config :cybercab, CybercabWeb.Endpoint,
    secret_key_base: env.("SECRET_KEY_BASE", String.duplicate("cybercab-benchmark-only-", 3))
end
