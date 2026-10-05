defmodule Kanban.Templates do
  @moduledoc """
  The pipelines the Rust kanban builds with `Multi`, as `Kanban.Multi` runs them: each in one
  transaction, and answering what each step made, by the names the Rust steps have.

  `layer` is `Kanban.Memory` or `Kanban.Sqlite`.
  """
  alias Kanban.Multi

  @doc "A board, with its To Do, In Progress and Done lists and a welcome card with a checklist."
  def board(layer, workspace_id, name, description \\ nil, opts \\ []) do
    boards = Module.concat(layer, Boards)
    opts = with_options(opts)

    Multi.run(resources(layer),
      board: fn _ -> boards.create_board(workspace_id, name, description, opts) end,
      todo_list: fn %{board: b} -> boards.create_list(b.id, "To Do", 0, opts) end,
      doing_list: fn %{board: b} -> boards.create_list(b.id, "In Progress", 1, opts) end,
      done_list: fn %{board: b} -> boards.create_list(b.id, "Done", 2, opts) end,
      welcome_card: fn %{board: b, todo_list: l} ->
        boards.create_card(
          b.id,
          l.id,
          "Welcome to your board!",
          "Explore lists, add cards, and track subtasks.",
          0,
          opts
        )
      end,
      task_1: fn %{welcome_card: c} ->
        boards.add_checklist_item(c.id, "Create your first custom card", 0, opts)
      end,
      task_2: fn %{welcome_card: c} ->
        boards.add_checklist_item(c.id, "Invite team members to workspace", 1, opts)
      end
    )
  end

  @doc "A workspace, and its owner enrolled as an admin."
  def workspace_with_owner(layer, owner_id, name, slug, opts \\ []) do
    workspaces = Module.concat(layer, Workspaces)
    opts = with_options(opts)

    Multi.run(resources(layer),
      workspace: fn _ -> workspaces.create_workspace(name, slug, opts) end,
      membership: fn %{workspace: ws} -> workspaces.add_member(ws.id, owner_id, :admin, opts) end
    )
  end

  @doc "A card moved to another list and, if given a note, a comment saying why."
  def move_card(layer, card, list_id, position, note \\ nil, opts \\ []) do
    boards = Module.concat(layer, Boards)
    opts = with_options(opts)

    steps = [moved_card: fn _ -> boards.move_card(card, list_id, position, opts) end]

    steps =
      if note do
        steps ++ [movement_comment: fn _ -> boards.add_comment(card.id, note, opts) end]
      else
        steps
      end

    Multi.run(resources(layer), steps)
  end

  @doc "Checklist items, in order, on a card."
  def add_checklist_items(layer, card_id, titles, opts \\ []) do
    boards = Module.concat(layer, Boards)
    opts = with_options(opts)

    steps =
      titles
      |> Enum.with_index()
      |> Enum.map(fn {title, index} ->
        {:"item_#{index}", fn _ -> boards.add_checklist_item(card_id, title, index, opts) end}
      end)

    Multi.run(resources(layer), steps)
  end

  # A code interface refuses a bare empty list as its last argument: it can't tell options
  # from params.
  defp with_options(opts), do: Keyword.put_new(opts, :authorize?, true)

  defp resources(layer) do
    for name <- ~w(User Workspace WorkspaceMember Board List Card ChecklistItem Comment)a,
        do: Module.concat(layer, name)
  end
end
