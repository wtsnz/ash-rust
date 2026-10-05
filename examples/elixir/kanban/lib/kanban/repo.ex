defmodule Kanban.Repo do
  @moduledoc "The SQLite database behind `Kanban.Sqlite`."
  use AshSqlite.Repo, otp_app: :kanban

  # AshSqlite doesn't wrap actions in transactions unless the repo says it may; ash-rust's
  # SQLite always does. One connection, as configured, keeps a transaction from being
  # locked out by another.
  @impl true
  def write_transactions?, do: true
end
