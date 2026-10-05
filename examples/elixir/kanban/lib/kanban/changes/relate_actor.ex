defmodule Kanban.Changes.RelateActor do
  @moduledoc """
  Sets `attribute` to the actor's id, as the Rust desk's `relate_actor`. With no actor it
  leaves the attribute alone.
  """
  use Ash.Resource.Change

  @impl true
  def change(changeset, opts, %{actor: %{id: id}}),
    do: Ash.Changeset.force_change_attribute(changeset, opts[:attribute], id)

  def change(changeset, _opts, _context), do: changeset
end
