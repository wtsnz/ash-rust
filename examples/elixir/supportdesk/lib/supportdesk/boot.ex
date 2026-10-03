defmodule Supportdesk.Boot do
  @moduledoc """
  Starts the desk as the Rust one does: migrates the database and, given a fixture,
  empties the desk's tables and loads it. It runs while the application starts, and
  `/health` answers only after it.
  """

  @tables ~w(audit_events ticket_tags comments tickets tags agents orgs)

  @doc "Whether the desk is ready: migrated, and the fixture loaded."
  def ready?, do: :persistent_term.get(__MODULE__, nil) != nil

  @doc "How long loading the fixture took, in milliseconds."
  def seeded_ms, do: :persistent_term.get(__MODULE__, 0)

  def child_spec(fixture),
    do: %{id: __MODULE__, start: {__MODULE__, :run, [fixture]}, restart: :temporary}

  def run(fixture) do
    {:ok, _, _} =
      Ecto.Migrator.with_repo(Supportdesk.Repo, &Ecto.Migrator.run(&1, :up, all: true), pool_size: 2)

    seeded_ms =
      if fixture do
        started = System.monotonic_time(:millisecond)
        Supportdesk.Repo.query!("TRUNCATE #{Enum.map_join(@tables, ", ", &~s("#{&1}"))} CASCADE")
        fixture |> File.read!() |> Jason.decode!() |> Supportdesk.Fixture.load()
        System.monotonic_time(:millisecond) - started
      else
        0
      end

    IO.puts("  Loaded #{fixture || "no fixture"} in #{seeded_ms} ms")
    :persistent_term.put(__MODULE__, seeded_ms)
    :ignore
  end
end
