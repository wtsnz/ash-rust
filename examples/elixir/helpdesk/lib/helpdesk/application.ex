defmodule Helpdesk.Application do
  @moduledoc false
  use Application

  @impl true
  def start(_type, _args) do
    children = [Helpdesk.Repo, Helpdesk.IntakeStore]
    Supervisor.start_link(children, strategy: :one_for_one, name: Helpdesk.Supervisor)
  end
end
