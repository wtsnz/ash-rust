defmodule Kanban.Multi do
  @moduledoc """
  Named steps run in order inside one transaction, each given what the steps before it
  made: the Rust desk's `Multi`. A step returns `{:ok, value}` or `{:error, error}`; the
  first error rolls the transaction back and answers `{:error, {step, error}}`. Where the
  data layer has no transactions (ETS), what was written before the error stays.

      Kanban.Multi.run([Board, List], [
        board: fn _ -> Kanban.Memory.Boards.create_board(workspace_id, "Roadmap") end,
        list: fn %{board: board} -> Kanban.Memory.Boards.create_list(board.id, "To Do", 0) end
      ])
  """

  defmodule Failure do
    @moduledoc "A step's error, raised to roll the transaction back."
    defexception [:step, :error]

    @impl true
    def message(%{step: step, error: error}), do: "step #{step} failed: #{inspect(error)}"
  end

  @type step :: {atom(), (map() -> {:ok, term()} | {:error, term()})}

  @spec run([module()], [step()]) :: {:ok, map()} | {:error, {atom(), term()}}
  def run(resources, steps) do
    # Ash.transact turns an `{:error, term}` into an Ash error, which would bury the step's
    # name and error; raising rolls back all the same, and the step's own error comes through.
    {:ok, results} =
      Ash.transact(resources, fn ->
        Enum.reduce(steps, %{}, fn {name, step}, results ->
          case step.(results) do
            {:ok, value} -> Map.put(results, name, value)
            {:error, error} -> raise Failure, step: name, error: error
          end
        end)
      end)

    {:ok, results}
  rescue
    failure in Failure -> {:error, {failure.step, failure.error}}
  end
end
