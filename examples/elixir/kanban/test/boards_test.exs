defmodule Kanban.BoardsTest do
  @moduledoc "Ports the Rust kanban's `tests/kanban_board.rs`, on ETS."
  use Kanban.KanbanCase

  alias Kanban.Memory.{Boards, Workspaces}
  alias Kanban.Memory.{Board, Card, List}
  alias Kanban.Templates

  test "a board template is built in one pipeline, and its aggregates count it" do
    alice = Workspaces.register_user!("Alice", "alice@test.com")
    as_alice = user_actor(alice)
    ws = Workspaces.create_workspace!("Product Org", "prod-org", actor: as_alice)

    {:ok, steps} =
      Templates.board(
        Kanban.Memory,
        ws.id,
        "Mobile App v2",
        "Design and development of Mobile App v2",
        actor: as_alice
      )

    assert steps.board.name == "Mobile App v2"
    assert steps.todo_list.title == "To Do"
    assert steps.doing_list.title == "In Progress"
    assert steps.done_list.title == "Done"
    assert steps.welcome_card.title == "Welcome to your board!"
    assert steps.task_1.title == "Create your first custom card"
    assert steps.task_2.title == "Invite team members to workspace"

    board =
      Board
      |> Ash.Query.load([:list_count, :card_count, :open_card_count])
      |> Ash.read_one!()

    assert {board.list_count, board.card_count, board.open_card_count} == {3, 1, 1}

    todo = Ash.get!(List, steps.todo_list.id, load: [:card_count, :open_card_count, :has_cards])
    assert {todo.card_count, todo.open_card_count, todo.has_cards} == {1, 1, true}

    doing = Ash.get!(List, steps.doing_list.id, load: [:card_count, :has_cards])
    assert {doing.card_count, doing.has_cards} == {0, false}

    card =
      Card
      |> Ash.Query.load([
        :title_length,
        :checklist_count,
        :completed_checklist_count,
        :comment_count
      ])
      |> Ash.read_one!()

    assert card.checklist_count == 2
    assert card.completed_checklist_count == 0
    assert card.comment_count == 0
    assert card.title_length == String.length("Welcome to your board!")

    Boards.toggle_checklist_item!(steps.task_1, true)

    card =
      Card
      |> Ash.Query.load([:checklist_count, :completed_checklist_count])
      |> Ash.read_one!()

    assert card.checklist_count == 2
    assert card.completed_checklist_count == 1
  end

  test "a card moves to another list with an activity comment, in one pipeline" do
    as_user = Kanban.Actor.user(Ash.UUID.generate())

    board = Boards.create_board!(Ash.UUID.generate(), "Sprint 42", nil, actor: as_user)
    todo = Boards.create_list!(board.id, "To Do", 0, actor: as_user)
    doing = Boards.create_list!(board.id, "In Progress", 1, actor: as_user)

    card =
      Boards.create_card!(
        board.id,
        todo.id,
        "Implement OAuth",
        "Add Google and GitHub providers",
        0,
        actor: as_user
      )

    assert card.list_id == todo.id
    assert card.creator_id == as_user.id

    {:ok, %{moved_card: moved, movement_comment: comment}} =
      Templates.move_card(
        Kanban.Memory,
        card,
        doing.id,
        0,
        "Alice started working on OAuth",
        actor: as_user
      )

    assert moved.list_id == doing.id
    assert comment.body == "Alice started working on OAuth"
    assert comment.author_id == as_user.id

    reloaded = Ash.get!(Card, card.id, load: [:comment_count])
    assert reloaded.list_id == doing.id
    assert reloaded.comment_count == 1

    assert Ash.get!(List, todo.id, load: [:card_count]).card_count == 0
    assert Ash.get!(List, doing.id, load: [:card_count]).card_count == 1
  end

  test "checklist items are added in order" do
    board = Boards.create_board!(Ash.UUID.generate(), "Sprint", nil)
    list = Boards.create_list!(board.id, "To Do", 0)
    card = Boards.create_card!(board.id, list.id, "Ship it", nil, 0)

    {:ok, items} = Templates.add_checklist_items(Kanban.Memory, card.id, ["a", "b", "c"])
    assert Enum.map(Map.values(items), & &1.position) |> Enum.sort() == [0, 1, 2]
    assert items.item_1.title == "b"
  end

  test "lists, cards and boards are renamed, moved, assigned and archived" do
    board = Boards.create_board!(Ash.UUID.generate(), "Sprint", "desc")
    list = Boards.create_list!(board.id, "To Do", 0)
    card = Boards.create_card!(board.id, list.id, "Ship it", nil, 0)
    assignee = Ash.UUID.generate()

    assert Boards.rename_list!(list, "Backlog").title == "Backlog"
    assert Boards.move_list!(list, 4).position == 4
    assert Boards.assign_card!(card, assignee).assignee_id == assignee
    assert Boards.assign_card!(card, nil).assignee_id == nil
    assert Boards.archive_card!(card).archived
    assert Boards.archive_list!(list).archived
    assert Boards.archive_board!(board).archived

    refute card.archived
    assert {:ok, _} = Boards.get_board(board.id)
  end

  test "a list's position can't be negative, and a card needs a title" do
    board = Boards.create_board!(Ash.UUID.generate(), "Sprint", nil)
    assert {:error, %Ash.Error.Invalid{}} = Boards.create_list(board.id, "To Do", -1)
    list = Boards.create_list!(board.id, "To Do", 0)
    assert {:error, %Ash.Error.Invalid{}} = Boards.move_list(list, -2)
    assert {:error, %Ash.Error.Invalid{}} = Boards.create_card(board.id, list.id, "", nil, 0)
    assert {:error, %Ash.Error.Invalid{}} = Boards.create_board(Ash.UUID.generate(), nil, nil)
  end

  test "comments are edited and deleted" do
    board = Boards.create_board!(Ash.UUID.generate(), "Sprint", nil)
    list = Boards.create_list!(board.id, "To Do", 0)
    card = Boards.create_card!(board.id, list.id, "Ship it", nil, 0)

    comment = Boards.add_comment!(card.id, "first")
    assert Boards.update_comment!(comment, "edited").body == "edited"
    assert {:error, %Ash.Error.Invalid{}} = Boards.update_comment(comment, nil)
    Boards.delete_comment!(comment)
    assert Boards.list_comments!() == []
  end
end
