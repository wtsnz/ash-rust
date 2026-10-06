import Config

# The same settings as the Rust desk, from the same environment variables.
env = fn name, default -> System.get_env(name, default) end

config :supportdesk, Supportdesk.Repo,
  url: env.("DATABASE_URL", "postgres://postgres:postgres@127.0.0.1:5432/supportdesk_elixir")

# The pool's size and queue, from the environment: what the saturation benchmark varies. Unset,
# they are the desk's own (20 connections) and Ecto's defaults (`queue_target` 50 ms, doubled
# once every checkout in a `queue_interval` of 2 s has waited longer, and requests past it
# dropped; `timeout` 15 s). `POOL_QUEUE_TARGET_MS` very high stops the dropping: requests queue.
integer = fn name ->
  case System.get_env(name) do
    nil -> nil
    value -> String.to_integer(value)
  end
end

pool =
  [
    pool_size: integer.("POOL_SIZE"),
    queue_target: integer.("POOL_QUEUE_TARGET_MS"),
    queue_interval: integer.("POOL_QUEUE_INTERVAL_MS"),
    timeout: integer.("POOL_TIMEOUT_MS")
  ]
  |> Enum.reject(fn {_, value} -> is_nil(value) end)

config :supportdesk, Supportdesk.Repo, pool

config :supportdesk, SupportdeskWeb.Endpoint,
  http: [ip: {127, 0, 0, 1}, port: String.to_integer(env.("PORT", "4000"))]

config :supportdesk, :fixture, System.get_env("FIXTURE")

if config_env() == :prod do
  config :supportdesk, SupportdeskWeb.Endpoint,
    secret_key_base: env.("SECRET_KEY_BASE", String.duplicate("supportdesk-benchmark-only-", 3))
end
