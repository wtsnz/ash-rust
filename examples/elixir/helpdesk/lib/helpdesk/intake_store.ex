defmodule Helpdesk.IntakeStore do
  @moduledoc """
  Where `intake` tickets go: out of the data layer, as the Rust desk's manual intake
  puts them in a store of its own.
  """
  use Agent

  def start_link(_opts), do: Agent.start_link(fn -> %{} end, name: __MODULE__)

  def put(ticket), do: Agent.update(__MODULE__, &Map.put(&1, ticket.id, ticket))

  def get(id), do: Agent.get(__MODULE__, &Map.get(&1, id))

  def clear, do: Agent.update(__MODULE__, fn _ -> %{} end)
end
