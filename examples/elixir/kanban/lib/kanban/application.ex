defmodule Kanban.Application do
  @moduledoc false
  use Application

  @impl true
  def start(_type, _args) do
    Supervisor.start_link([Kanban.Repo], strategy: :one_for_one, name: Kanban.Supervisor)
  end
end
