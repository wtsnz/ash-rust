defmodule Kanban.Actor do
  @moduledoc "Who acts on the boards: a user, or an admin."

  def user(id), do: %{id: id, role: "user"}
  def admin(id), do: %{id: id, role: "admin"}
end
