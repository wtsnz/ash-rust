defmodule Supportdesk.MixProject do
  use Mix.Project

  def project do
    [
      app: :supportdesk,
      version: "0.1.0",
      elixir: "~> 1.18",
      start_permanent: Mix.env() == :prod,
      deps: deps()
    ]
  end

  def application do
    [extra_applications: [:logger], mod: {Supportdesk.Application, []}]
  end

  defp deps do
    [
      {:ash, "~> 3.33"},
      {:ash_postgres, "~> 2.13"},
      {:ash_graphql, "~> 1.12"},
      {:ash_state_machine, "~> 0.2"},
      {:absinthe, "~> 1.12"},
      {:absinthe_plug, "~> 1.5"},
      {:absinthe_graphql_ws, "~> 0.3"},
      {:absinthe_phoenix, "~> 2.0"},
      {:phoenix, "~> 1.8"},
      {:phoenix_pubsub, "~> 2.3"},
      {:bandit, "~> 1.12"},
      {:jason, "~> 1.4"},
      {:picosat_elixir, "~> 0.2"}
    ]
  end
end
