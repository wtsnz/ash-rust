use ash_sqlite::Sqlite;
use kanban::kanban::board::fields as b_f;
use kanban::workspaces::workspace::fields as ws_f;
use kanban::{
    Board, Kanban, Workspaces, actor_user, board_template_multi,
    workspace_with_owner_multi,
};

#[tokio::test]
async fn test_sqlite_two_domains_and_transactions() {
    let db = Sqlite::memory().await.unwrap();

    let workspaces = Workspaces::new(db.clone());
    let kanban = Kanban::new(db);

    // Install schemas for both domains into SQLite
    workspaces.install().await.unwrap();
    kanban.install().await.unwrap();

    // 1. Create user and workspace in Workspaces domain
    let alice = workspaces.register_user("Alice", "alice@corp.com").await.unwrap();
    let as_alice_ws = workspaces.with_actor(actor_user(alice.id));

    let ws_multi = workspace_with_owner_multi(
        &as_alice_ws.ctx,
        alice.id,
        "Platform Infrastructure",
        "platform-infra",
    )
    .unwrap();
    let ws_res = as_alice_ws.run_multi(ws_multi).await.unwrap();
    let ws = ws_res.get::<kanban::Workspace>("workspace").unwrap();

    // 2. Create board in Kanban domain belonging to the workspace
    let as_alice_kb = kanban.with_actor(actor_user(alice.id));
    let board_multi = board_template_multi(
        &as_alice_kb.ctx,
        ws.id,
        "Kubernetes Migration",
        Some("Track migration to EKS"),
    )
    .unwrap();

    let kb_res = as_alice_kb.run_multi(board_multi).await.unwrap();
    let board = kb_res.get::<Board>("board").unwrap();
    assert_eq!(board.workspace_id, ws.id);

    // 3. Verify SQLite aggregates on both domains
    let loaded_ws = as_alice_ws
        .workspaces()
        .aggregate(ws_f::member_count)
        .one()
        .await
        .unwrap();
    assert_eq!(loaded_ws.member_count, Some(1));

    let loaded_board = as_alice_kb
        .boards()
        .aggregate(b_f::list_count)
        .aggregate(b_f::card_count)
        .one()
        .await
        .unwrap();
    assert_eq!(loaded_board.list_count, Some(3));
    assert_eq!(loaded_board.card_count, Some(1));

    // 4. Verify transaction rollback in SQLite
    let fail_multi = ash_core::Multi::new()
        .create(
            "temp_board",
            Board::create(&as_alice_kb.ctx)
                .workspace_id(ws.id)
                .name("Temporary Board")
                .changeset()
                .unwrap(),
        )
        .run("bomb", |_ctx, _res| async move {
            Err::<(), _>(ash_core::Error::Invalid("rollback test".into()))
        });

    let err = as_alice_kb.run_multi(fail_multi).await.unwrap_err();
    assert_eq!(err.multi_step(), Some("bomb"));

    // Verify "Temporary Board" does not exist in SQLite
    let boards = as_alice_kb.list_boards().all().await.unwrap();
    assert_eq!(boards.len(), 1);
    assert_eq!(boards[0].name, "Kubernetes Migration");
}
