defmodule AstroHelpdesk.Seed do
  @moduledoc "The Rust server's seed: two representatives and three tickets."

  alias AstroHelpdesk.Desk.{Representative, Ticket}

  def run do
    sarah =
      Ash.create!(Representative, %{
        name: "Sarah Chen",
        email: "sarah.chen@support.ash",
        role: "Support Lead"
      })

    david =
      Ash.create!(Representative, %{
        name: "David Kim",
        email: "david.kim@eng.ash",
        role: "Staff SRE"
      })

    open(%{
      title: "Postgres connection pool exhaustion during traffic spike",
      description: "API gateway latency spiked to 2.4s. Connection pool maxed at 50.",
      status: :in_progress,
      priority: 1,
      author_id: david.id
    })

    open(%{
      title: "Password reset token expires prematurely on mobile Safari",
      description:
        "Multiple customer reports of 401 token expired within 2 minutes of requesting.",
      status: :open,
      priority: 2,
      author_id: sarah.id
    })

    open(%{
      title: "Nightly customer SLA CSV report delivery timed out",
      description: "Cron job failed at 03:00 UTC. Email notification dispatched.",
      status: :resolved,
      priority: 4,
      author_id: sarah.id
    })

    :ok
  end

  defp open(input), do: Ash.create!(Ticket, input, action: :open)
end
