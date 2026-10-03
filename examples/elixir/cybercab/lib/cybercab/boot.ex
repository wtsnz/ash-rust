defmodule Cybercab.Boot do
  @moduledoc """
  Starts a shift as the Rust Cybercab does: migrates the database, empties the command
  center's tables, and seeds Austin. It runs while the application starts, before the
  simulation, and `/health` answers only after it.
  """

  alias Cybercab.Sim.{City, Seed}

  @tables ~w(telemetry_samples pulse_samples fleet_alerts trips cabs riders service_zones depots)

  @doc "Whether the shift has started: the city is seeded."
  def ready?, do: :persistent_term.get(__MODULE__, nil) == :ready

  def child_spec(config),
    do: %{id: __MODULE__, start: {__MODULE__, :run, [config]}, restart: :temporary}

  def run(config) do
    {:ok, _, _} =
      Ecto.Migrator.with_repo(Cybercab.Repo, &Ecto.Migrator.run(&1, :up, all: true), pool_size: 2)

    Cybercab.Repo.query!("TRUNCATE #{Enum.map_join(@tables, ", ", &~s("#{&1}"))} CASCADE")
    Seed.austin(City.get(), config.seed, config.fleet)
    :persistent_term.put(__MODULE__, :ready)
    :ignore
  end
end
