defmodule Kanban.SqliteTest do
  @moduledoc """
  Ports the Rust kanban's `tests/sqlite_cross_domain.rs`: two domains on one database, and a
  pipeline that rolls back. AshSqlite serves no resource aggregates, so the counts are
  read as the rows themselves.
  """
  use Kanban.KanbanCase

  alias Kanban.Sqlite.{Boards, Workspaces}
  alias Kanban.Templates

  test "two domains share a database, and a failed pipeline leaves nothing behind" do
    alice = Workspaces.register_user!("Alice", "alice@corp.com")
    as_alice = user_actor(alice)

    {:ok, %{workspace: ws}} =
      Templates.workspace_with_owner(
        Kanban.Sqlite,
        alice.id,
        "Platform Infrastructure",
        "platform-infra",
        actor: as_alice
      )

    {:ok, %{board: board}} =
      Templates.board(Kanban.Sqlite, ws.id, "Kubernetes Migration", "Track migration to EKS",
        actor: as_alice
      )

    assert board.workspace_id == ws.id
    assert length(Workspaces.list_members!()) == 1
    assert length(Boards.list_lists!()) == 3
    assert length(Boards.list_cards!()) == 1
    assert length(Boards.list_checklist_items!()) == 2

    failing =
      Kanban.Multi.run(
        [Kanban.Sqlite.Board],
        temp_board: fn _ ->
          Boards.create_board(ws.id, "Temporary Board", nil, actor: as_alice)
        end,
        bomb: fn _ -> {:error, :rollback_test} end
      )

    assert {:error, {:bomb, :rollback_test}} = failing

    assert [%{name: "Kubernetes Migration"}] = Boards.list_boards!()
  end

  test "a pipeline that fails in its own step rolls the earlier ones back" do
    alice = Workspaces.register_user!("Alice", "alice@corp.com")
    as_alice = user_actor(alice)

    # The workspace is made, then the membership is refused: its role isn't one of the three.
    assert {:error, {:membership, %Ash.Error.Invalid{}}} =
             Kanban.Multi.run(
               [Kanban.Sqlite.Workspace, Kanban.Sqlite.WorkspaceMember],
               workspace: fn _ ->
                 Workspaces.create_workspace("Doomed", "doomed", actor: as_alice)
               end,
               membership: fn %{workspace: ws} ->
                 Workspaces.add_member(ws.id, alice.id, "superadmin", actor: as_alice)
               end
             )

    assert Workspaces.list_workspaces!() == []
  end
end
