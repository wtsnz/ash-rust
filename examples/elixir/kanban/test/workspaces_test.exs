defmodule Kanban.WorkspacesTest do
  @moduledoc "Ports the Rust kanban's `tests/workspaces.rs`."
  use Kanban.KanbanCase

  alias Kanban.Memory.Workspaces
  alias Kanban.Templates

  test "a workspace is created with its owner as an admin, and counts its members" do
    alice = Workspaces.register_user!("Alice", "alice@test.com")
    bob = Workspaces.register_user!("Bob", "bob@test.com")
    carol = Workspaces.register_user!("Carol", "carol@test.com")
    as_alice = user_actor(alice)

    {:ok, %{workspace: ws, membership: membership}} =
      Templates.workspace_with_owner(Kanban.Memory, alice.id, "Design Team", "design-team",
        actor: as_alice
      )

    assert ws.name == "Design Team"
    assert ws.slug == "design-team"
    assert ws.owner_id == alice.id
    assert membership.role == :admin

    Workspaces.add_member!(ws.id, bob.id, "member", actor: as_alice)
    carol_member = Workspaces.add_member!(ws.id, carol.id, "guest", actor: as_alice)

    [loaded] = Ash.read!(Kanban.Memory.Workspace, load: [:member_count, :has_members])
    assert loaded.member_count == 3
    assert loaded.has_members == true

    Workspaces.remove_member!(carol_member, actor: as_alice)

    [updated] = Ash.read!(Kanban.Memory.Workspace, load: [:member_count])
    assert updated.member_count == 2
  end

  test "creating a workspace needs an actor" do
    assert {:error, %Ash.Error.Forbidden{}} = Workspaces.create_workspace("Eng", "eng")
  end

  test "a member's role must be one of admin, member and guest" do
    alice = Workspaces.register_user!("Alice", "alice@test.com")
    as_alice = user_actor(alice)
    ws = Workspaces.create_workspace!("Engineering", "eng", actor: as_alice)

    assert {:error,
            %Ash.Error.Invalid{
              errors: [%Ash.Error.Changes.InvalidAttribute{field: :role, value: "superadmin"}]
            }} = Workspaces.add_member(ws.id, alice.id, "superadmin", actor: as_alice)
  end

  test "a user needs a name and an email" do
    assert {:error, %Ash.Error.Invalid{}} = Workspaces.register_user(nil, "a@b.c")
    assert {:error, %Ash.Error.Invalid{}} = Workspaces.register_user("A", nil)
  end
end
