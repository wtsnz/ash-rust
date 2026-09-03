use ash_memory::Memory;
use kanban::kanban::board::fields as b_f;
use kanban::kanban::card::fields as c_f;
use kanban::kanban::list::fields as l_f;
use kanban::{
    Board, Card, ChecklistItem, Comment, Kanban, List, Workspaces, actor_user,
    board_template_multi, move_card_multi,
};

#[tokio::test]
async fn test_board_template_multi_and_aggregates() {
    let mem = Memory::new();
    let workspaces = Workspaces::new(mem.clone());
    let kanban = Kanban::new(mem);

    let alice = workspaces.register_user("Alice", "alice@test.com").await.unwrap();
    let as_alice = kanban.with_actor(actor_user(alice.id));

    let ws = workspaces
        .with_actor(actor_user(alice.id))
        .create_workspace("Product Org", "prod-org")
        .await
        .unwrap();

    // Run atomic board template multi pipeline
    let multi = board_template_multi(
        &as_alice.ctx,
        ws.id,
        "Mobile App v2",
        Some("Design and development of Mobile App v2"),
    )
    .unwrap();

    let results = as_alice.run_multi(multi).await.unwrap();

    let board = results.get::<Board>("board").unwrap();
    let todo_list = results.get::<List>("todo_list").unwrap();
    let doing_list = results.get::<List>("doing_list").unwrap();
    let done_list = results.get::<List>("done_list").unwrap();
    let welcome_card = results.get::<Card>("welcome_card").unwrap();
    let task_1 = results.get::<ChecklistItem>("task_1").unwrap();
    let task_2 = results.get::<ChecklistItem>("task_2").unwrap();

    assert_eq!(board.name, "Mobile App v2");
    assert_eq!(todo_list.title, "To Do");
    assert_eq!(doing_list.title, "In Progress");
    assert_eq!(done_list.title, "Done");
    assert_eq!(welcome_card.title, "Welcome to your board!");
    assert_eq!(task_1.title, "Create your first custom card");
    assert_eq!(task_2.title, "Invite team members to workspace");

    // Check Board aggregates
    let loaded_board = as_alice
        .boards()
        .aggregate(b_f::list_count)
        .aggregate(b_f::card_count)
        .aggregate(b_f::open_card_count)
        .one()
        .await
        .unwrap();

    assert_eq!(loaded_board.list_count, Some(3));
    assert_eq!(loaded_board.card_count, Some(1));
    assert_eq!(loaded_board.open_card_count, Some(1));

    // Check List aggregates
    let loaded_todo = as_alice
        .lists()
        .filter(l_f::id.eq(todo_list.id))
        .aggregate(l_f::card_count)
        .aggregate(l_f::open_card_count)
        .aggregate(l_f::has_cards)
        .one()
        .await
        .unwrap();
    assert_eq!(loaded_todo.card_count, Some(1));
    assert_eq!(loaded_todo.open_card_count, Some(1));
    assert_eq!(loaded_todo.has_cards, Some(true));

    let loaded_doing = as_alice
        .lists()
        .filter(l_f::id.eq(doing_list.id))
        .aggregate(l_f::card_count)
        .aggregate(l_f::has_cards)
        .one()
        .await
        .unwrap();
    assert_eq!(loaded_doing.card_count, Some(0));
    assert_eq!(loaded_doing.has_cards, Some(false));

    // Check Card calculations and aggregates
    let loaded_card = as_alice
        .cards()
        .calc(c_f::title_length)
        .aggregate(c_f::checklist_count)
        .aggregate(c_f::completed_checklist_count)
        .aggregate(c_f::comment_count)
        .one()
        .await
        .unwrap();

    assert_eq!(loaded_card.checklist_count, Some(2));
    assert_eq!(loaded_card.completed_checklist_count, Some(0));
    assert_eq!(loaded_card.comment_count, Some(0));
    assert_eq!(
        loaded_card.title_length,
        Some("Welcome to your board!".len() as i64)
    );

    // Toggle task 1 completed
    as_alice.toggle_checklist_item(task_1, true).await.unwrap();

    let updated_card = as_alice
        .cards()
        .aggregate(c_f::checklist_count)
        .aggregate(c_f::completed_checklist_count)
        .one()
        .await
        .unwrap();

    assert_eq!(updated_card.checklist_count, Some(2));
    assert_eq!(updated_card.completed_checklist_count, Some(1)); // 1 completed!
}

#[tokio::test]
async fn test_move_card_multi_and_activity_comment() {
    let mem = Memory::new();
    let user_id = uuid::Uuid::new_v4();
    let kanban = Kanban::new(mem).with_actor(actor_user(user_id));

    let board = kanban
        .create_board(uuid::Uuid::new_v4(), "Sprint 42", None)
        .await
        .unwrap();

    let todo_list = kanban.create_list(board.id, "To Do", 0).await.unwrap();
    let doing_list = kanban
        .create_list(board.id, "In Progress", 1)
        .await
        .unwrap();

    let card = kanban
        .create_card(
            board.id,
            todo_list.id,
            "Implement OAuth",
            Some("Add Google and GitHub providers".into()),
            0,
        )
        .await
        .unwrap();

    assert_eq!(card.list_id, todo_list.id);

    // Move card from To Do to In Progress with an activity comment atomically
    let move_multi = move_card_multi(
        &kanban.ctx,
        card.clone(),
        doing_list.id,
        0,
        Some("Alice started working on OAuth".into()),
    )
    .unwrap();

    let res = kanban.run_multi(move_multi).await.unwrap();
    let moved_card = res.get::<Card>("moved_card").unwrap();
    let comment = res.get::<Comment>("movement_comment").unwrap();

    assert_eq!(moved_card.list_id, doing_list.id);
    assert_eq!(comment.body, "Alice started working on OAuth");

    // Verify card in Doing list has 1 comment
    let reloaded = kanban
        .cards()
        .filter(c_f::id.eq(card.id))
        .aggregate(c_f::comment_count)
        .one()
        .await
        .unwrap();
    assert_eq!(reloaded.list_id, doing_list.id);
    assert_eq!(reloaded.comment_count, Some(1));

    // Verify list card counts
    let todo_aggs = kanban
        .lists()
        .filter(l_f::id.eq(todo_list.id))
        .aggregate(l_f::card_count)
        .one()
        .await
        .unwrap();
    assert_eq!(todo_aggs.card_count, Some(0));

    let doing_aggs = kanban
        .lists()
        .filter(l_f::id.eq(doing_list.id))
        .aggregate(l_f::card_count)
        .one()
        .await
        .unwrap();
    assert_eq!(doing_aggs.card_count, Some(1));
}
