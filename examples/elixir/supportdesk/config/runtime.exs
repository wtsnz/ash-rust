import Config

# The same settings as the Rust desk, from the same environment variables.
env = fn name, default -> System.get_env(name, default) end

config :supportdesk, Supportdesk.Repo,
  url: env.("DATABASE_URL", "postgres://postgres:postgres@127.0.0.1:5432/supportdesk_elixir")

config :supportdesk, SupportdeskWeb.Endpoint,
  http: [ip: {127, 0, 0, 1}, port: String.to_integer(env.("PORT", "4000"))]

config :supportdesk, :fixture, System.get_env("FIXTURE")

if config_env() == :prod do
  config :supportdesk, SupportdeskWeb.Endpoint,
    secret_key_base: env.("SECRET_KEY_BASE", String.duplicate("supportdesk-benchmark-only-", 3))
end
