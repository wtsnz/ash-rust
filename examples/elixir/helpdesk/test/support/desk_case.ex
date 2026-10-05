defmodule Helpdesk.DeskCase do
  @moduledoc """
  The desk's tests, run once for each data layer: `use Helpdesk.DeskCase, layer: Helpdesk.Memory`
  runs them on ETS, `layer: Helpdesk.Sqlite` on SQLite. They port the Rust helpdesk's
  `tests/helpdesk.rs` and `tests/sqlite.rs`.
  """
  use ExUnit.CaseTemplate

  using opts do
    layer = Keyword.fetch!(opts, :layer)

    quote do
      use ExUnit.Case, async: false

      import Helpdesk.DeskCase
      require Ash.Query

      @desk unquote(layer)
      @ticket Module.concat(unquote(layer), Ticket)
      @representative Module.concat(unquote(layer), Representative)

      setup do
        Helpdesk.DeskCase.reset()
        :ok
      end
    end
  end

  @doc "Empties every store a test could have written to."
  def reset do
    for resource <- [
          Helpdesk.Memory.Ticket,
          Helpdesk.Memory.Representative,
          Helpdesk.Orders.LineItem,
          Helpdesk.Orders.Order
        ] do
      Ash.DataLayer.Ets.stop(resource)
    end

    Helpdesk.Repo.delete_all(Helpdesk.Sqlite.Ticket)
    Helpdesk.Repo.delete_all(Helpdesk.Sqlite.Representative)
    Helpdesk.IntakeStore.clear()
  end

  require Ash.Query

  @doc "Tickets whose subject is longer than `length`, by filtering on the calculation."
  def longer_than(query, length), do: Ash.Query.filter(query, subject_length > ^length)

  def customer, do: Helpdesk.Actor.customer(Ash.UUID.generate())
  def representative(rep), do: Helpdesk.Actor.representative(rep.id)
end
