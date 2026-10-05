defmodule Helpdesk.Intake do
  @moduledoc "The `intake` create: the ticket is made, then kept in `Helpdesk.IntakeStore`."
  use Ash.Resource.ManualCreate

  @impl true
  def create(changeset, _opts, _context) do
    with {:ok, ticket} <- Ash.Changeset.apply_attributes(changeset) do
      Helpdesk.IntakeStore.put(ticket)
      {:ok, ticket}
    end
  end
end
