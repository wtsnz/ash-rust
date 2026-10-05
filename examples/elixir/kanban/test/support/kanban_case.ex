defmodule Kanban.KanbanCase do
  @moduledoc "Tests on a clean slate: every ETS table and SQLite table emptied first."
  use ExUnit.CaseTemplate

  @names ~w(User Workspace WorkspaceMember Board List Card ChecklistItem Comment)a

  using do
    quote do
      require Ash.Query
      import Kanban.KanbanCase
    end
  end

  setup do
    for name <- @names do
      Ash.DataLayer.Ets.stop(Module.concat(Kanban.Memory, name))
    end

    # Children first, for the foreign keys between them.
    for name <- ~w(Comment ChecklistItem Card List Board WorkspaceMember Workspace User)a do
      Kanban.Repo.delete_all(Module.concat(Kanban.Sqlite, name))
    end

    :ok
  end

  def user_actor(user), do: Kanban.Actor.user(user.id)
end
