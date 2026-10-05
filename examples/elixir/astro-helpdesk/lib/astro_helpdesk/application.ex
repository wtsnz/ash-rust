defmodule AstroHelpdesk.Application do
  @moduledoc """
  The helpdesk's GraphQL server: seeded as the Rust one is, and serving the same API at the
  same paths, so the Astro frontend runs against either.
  """
  use Application

  @impl true
  def start(_type, _args) do
    children = [
      {Phoenix.PubSub, name: AstroHelpdesk.PubSub},
      AstroHelpdeskWeb.Endpoint,
      {Absinthe.Subscription, AstroHelpdeskWeb.Endpoint}
    ]

    with {:ok, supervisor} <-
           Supervisor.start_link(children, strategy: :one_for_one, name: AstroHelpdesk.Supervisor) do
      if Application.get_env(:astro_helpdesk, :seed?, true), do: AstroHelpdesk.Seed.run()
      {:ok, supervisor}
    end
  end
end
