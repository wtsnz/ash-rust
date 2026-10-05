defmodule Helpdesk.Actor do
  @moduledoc "Who acts on the desk: a customer, or a representative."

  def customer(id), do: %{id: id, role: "customer"}
  def representative(id), do: %{id: id, role: "representative"}
end
