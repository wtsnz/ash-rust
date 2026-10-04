defmodule Supportdesk.Application do
  @moduledoc """
  The desk: the database, the API, and the fixture loaded before the API reports healthy.
  """
  use Application

  @impl true
  def start(_type, _args) do
    children = [
      Supportdesk.Repo,
      {Phoenix.PubSub, name: Supportdesk.PubSub},
      SupportdeskWeb.Endpoint,
      {Absinthe.Subscription, SupportdeskWeb.Endpoint},
      AshGraphql.Subscription.Batcher,
      # `/health` answers once this has loaded the fixture.
      {Supportdesk.Boot, Application.get_env(:supportdesk, :fixture)}
    ]

    Supervisor.start_link(children, strategy: :one_for_one, name: Supportdesk.Supervisor)
  end
end
