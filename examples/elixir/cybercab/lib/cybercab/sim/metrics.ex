defmodule Cybercab.Sim.Metrics do
  @moduledoc """
  How the simulation's ticks are going, for `GET /metrics`: how many have run, how many
  failed, and how long the most recent took. Kept in a public ETS table, so reading it
  never waits on a tick.
  """

  @table __MODULE__
  # Ticks `/metrics` reports the durations of.
  @recent_ticks 120

  def child_spec(_), do: %{id: __MODULE__, start: {__MODULE__, :start_link, []}}

  def start_link do
    Agent.start_link(
      fn ->
        :ets.new(@table, [:named_table, :public, :set, read_concurrency: true])
        :ets.insert(@table, {:ticks, 0, 0, []})
      end,
      name: __MODULE__
    )
  end

  def record(elapsed_ms, ok?) do
    [{:ticks, ticks, errors, recent}] = :ets.lookup(@table, :ticks)
    recent = Enum.take([elapsed_ms | recent], @recent_ticks)
    :ets.insert(@table, {:ticks, ticks + 1, if(ok?, do: errors, else: errors + 1), recent})
  end

  @doc "`{ ticks, errors, recent_tick_ms }`, as both Cybercab servers report them."
  def to_map do
    [{:ticks, ticks, errors, recent}] = :ets.lookup(@table, :ticks)
    %{ticks: ticks, errors: errors, recent_tick_ms: Enum.reverse(recent)}
  end
end
