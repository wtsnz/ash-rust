defmodule Supportdesk.Fixture do
  @moduledoc """
  Loads the fixture the Rust desk's `fixture` binary writes, through each resource's
  `seed` action: one array of records per resource, attributes by name.
  """

  alias Supportdesk.Desk.{Agent, Comment, Org, Tag, Ticket, TicketTag}

  def load(fixture) do
    seed(Org, fixture["orgs"], nil)

    fixture["orgs"]
    |> Enum.map(& &1["slug"])
    |> Enum.each(fn org ->
      of_org = fn key -> Enum.filter(fixture[key], &(&1["org"] == org)) end
      seed(Agent, of_org.("agents"), org)
      seed(Tag, of_org.("tags"), org)
      seed(Ticket, of_org.("tickets"), org)
      seed(Comment, of_org.("comments"), org)
      seed(TicketTag, of_org.("ticket_tags"), org)
    end)
  end

  defp seed(resource, records, tenant) do
    %Ash.BulkResult{status: :success} =
      Ash.bulk_create(records, resource, :seed,
        tenant: tenant,
        authorize?: false,
        batch_size: 1_000,
        return_errors?: true,
        stop_on_error?: true
      )
  end
end
