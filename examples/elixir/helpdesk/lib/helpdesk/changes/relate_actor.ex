defmodule Helpdesk.Changes.RelateActor do
  @moduledoc "Makes the actor the ticket's opener, as the Rust desk's `relate_actor(opener_id)`."
  use Ash.Resource.Change

  @impl true
  def change(changeset, _opts, %{actor: %{id: id}}),
    do: Ash.Changeset.force_change_attribute(changeset, :opener_id, id)

  def change(changeset, _opts, _context), do: changeset
end
