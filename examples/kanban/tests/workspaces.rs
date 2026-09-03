use ash_core::Error;
use ash_memory::Memory;
use kanban::workspaces::workspace::fields as ws_f;
use kanban::{Workspace, Workspaces, actor_user, workspace_with_owner_multi};

#[tokio::test]
async fn test_workspace_creation_and_membership_aggregates() {
    let mem = Memory::new();
    let workspaces = Workspaces::new(mem);

    // 1. Register users
    let alice = workspaces.register_user("Alice", "alice@test.com").await.unwrap();
    let bob = workspaces.register_user("Bob", "bob@test.com").await.unwrap();
    let carol = workspaces.register_user("Carol", "carol@test.com").await.unwrap();

    let as_alice = workspaces.with_actor(actor_user(alice.id));

    // 2. Atomic workspace + owner enrollment
    let multi = workspace_with_owner_multi(&as_alice.ctx, alice.id, "Design Team", "design-team").unwrap();
    let res = as_alice.run_multi(multi).await.unwrap();
    let ws = res.get::<Workspace>("workspace").unwrap();
    assert_eq!(ws.name, "Design Team");
    assert_eq!(ws.slug, "design-team");
    assert_eq!(ws.owner_id, alice.id);

    // 3. Add Bob (member) and Carol (guest)
    let _m_bob = as_alice
        .add_member(ws.id, bob.id, "member")
        .await
        .unwrap();
    let m_carol = as_alice
        .add_member(ws.id, carol.id, "guest")
        .await
        .unwrap();

    // 4. Query workspace with aggregates (member_count, has_members)
    let loaded_ws = as_alice
        .workspaces()
        .aggregate(ws_f::member_count)
        .aggregate(ws_f::has_members)
        .one()
        .await
        .unwrap();

    assert_eq!(loaded_ws.member_count, Some(3)); // Alice + Bob + Carol
    assert_eq!(loaded_ws.has_members, Some(true));

    // 5. Remove Carol
    as_alice.remove_member(&m_carol).await.unwrap();

    let updated_ws = as_alice
        .workspaces()
        .aggregate(ws_f::member_count)
        .one()
        .await
        .unwrap();
    assert_eq!(updated_ws.member_count, Some(2));
}

#[tokio::test]
async fn test_workspace_member_role_validation() {
    let mem = Memory::new();
    let workspaces = Workspaces::new(mem);

    let alice = workspaces.register_user("Alice", "alice@test.com").await.unwrap();
    let as_alice = workspaces.with_actor(actor_user(alice.id));

    let ws = as_alice.create_workspace("Engineering", "eng").await.unwrap();

    // Invalid role "superadmin" should fail validation
    let err = as_alice
        .add_member(ws.id, alice.id, "superadmin")
        .await
        .unwrap_err();

    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "role");
            assert!(message.contains("one of"));
        }
        other => panic!("expected Error::Validation, got {:?}", other),
    }
}
