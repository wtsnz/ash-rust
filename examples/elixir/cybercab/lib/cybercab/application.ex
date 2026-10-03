defmodule Cybercab.Application do
  @moduledoc """
  The command center: the database, the API, the shift's seed and the simulated fleet.
  The API reports itself healthy only once the city is seeded, so clients wait for it.
  """
  use Application

  @impl true
  def start(_type, _args) do
    sim = Map.new(Application.fetch_env!(:cybercab, :simulation))

    children =
      [
        Cybercab.Repo,
        {Phoenix.PubSub, name: Cybercab.PubSub},
        Cybercab.Sim.Metrics,
        CybercabWeb.Endpoint,
        {Absinthe.Subscription, CybercabWeb.Endpoint},
        AshGraphql.Subscription.Batcher,
        # `/health` answers once this has seeded the city.
        {Cybercab.Boot, sim}
      ] ++ if(sim.enabled?, do: [{Cybercab.Sim, sim}], else: [])

    Supervisor.start_link(children, strategy: :one_for_one, name: Cybercab.Supervisor)
  end
end
