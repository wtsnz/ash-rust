use ash_core::{AshEnum, AshType, Context, Resource, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum PostStatus {
    Draft,
    #[ash(string = "in_progress")]
    InProgress,
    #[ash(rename = "published")]
    Published,
    Archived,
}

#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum Priority {
    Low,
    Medium,
    High,
}

resource! {
Post {
    table "posts";

attributes {
    id: Uuid [pk];
    title: String;
    status: PostStatus [default: PostStatus::Draft];
    priority: Option<Priority> [enum];
}

actions {
    create create {
        primary;
        accept [title, status, priority];
    }

    update update {
        primary;
        accept [title, status, priority];
    }

    read read {
        primary;
    }
}
}}

#[tokio::test]
async fn test_ash_enum_traits_and_conversions() {
    // 1. INVARIANT: Variants array is exposed with correct snake_case and renames
    assert_eq!(
        PostStatus::VARIANTS,
        &["draft", "in_progress", "published", "archived"]
    );
    assert_eq!(Priority::VARIANTS, &["low", "medium", "high"]);

    // 2. INVARIANT: as_str and Display output matching strings
    assert_eq!(PostStatus::Draft.as_str(), "draft");
    assert_eq!(PostStatus::InProgress.as_str(), "in_progress");
    assert_eq!(PostStatus::Published.as_str(), "published");
    assert_eq!(format!("{}", PostStatus::Archived), "archived");

    // 3. INVARIANT: Parse from string / FromStr
    assert_eq!(
        PostStatus::parse("in_progress").unwrap(),
        PostStatus::InProgress
    );
    assert_eq!(
        "published".parse::<PostStatus>().unwrap(),
        PostStatus::Published
    );
    assert!(PostStatus::parse("nonexistent").is_err());

    // 4. INVARIANT: AshType attr_type metadata is Atom
    let attr_type = <PostStatus as AshType>::ATTR_TYPE;
    assert_eq!(
        attr_type,
        ash_core::AttrType::Atom {
            one_of: &["draft", "in_progress", "published", "archived"]
        }
    );
}

#[tokio::test]
async fn test_ash_enum_in_memory_resource() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    // 1. Create with default enum status:
    let p1 = Post::create(&ctx)
        .title("First Post")
        .priority(Priority::High)
        .await
        .unwrap();

    assert_eq!(p1.status, PostStatus::Draft, "default enum value applied");
    assert_eq!(p1.priority, Some(Priority::High));

    // 2. Create with explicit enum status:
    let p2 = Post::create(&ctx)
        .title("Second Post")
        .status(PostStatus::Published)
        .await
        .unwrap();

    assert_eq!(p2.status, PostStatus::Published);
    assert_eq!(p2.priority, None);

    // 3. Type-safe filtering using Post::status.eq(PostStatus)
    let published_posts = Post::query(&ctx)
        .filter(Post::status.eq(PostStatus::Published))
        .all()
        .await
        .unwrap();

    assert_eq!(published_posts.len(), 1);
    assert_eq!(published_posts[0].id, p2.id);

    // 4. Update enum attribute
    let updated_p1 = Post::update(&ctx, p1.id)
        .status(PostStatus::InProgress)
        .priority(Priority::Medium)
        .await
        .unwrap();

    assert_eq!(updated_p1.status, PostStatus::InProgress);
    assert_eq!(updated_p1.priority, Some(Priority::Medium));
}

#[tokio::test]
async fn test_ash_enum_in_sqlite_resource() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Post::DEF]).await.unwrap();
    let ctx = Context::new(sqlite);

    // 1. Create with enum
    let post = Post::create(&ctx)
        .title("SQLite Post")
        .status(PostStatus::InProgress)
        .priority(Priority::Low)
        .await
        .unwrap();

    assert_eq!(post.status, PostStatus::InProgress);
    assert_eq!(post.priority, Some(Priority::Low));

    // 2. Reload from SQLite and ensure deserialization matches native enum
    let reloaded = Post::get(&ctx, post.id).await.unwrap();
    assert_eq!(reloaded.status, PostStatus::InProgress);
    assert_eq!(reloaded.priority, Some(Priority::Low));

    // 3. Filter in SQLite SQL query
    let in_progress = Post::query(&ctx)
        .filter(Post::status.eq(PostStatus::InProgress))
        .all()
        .await
        .unwrap();

    assert_eq!(in_progress.len(), 1);
    assert_eq!(in_progress[0].id, post.id);

    let published = Post::query(&ctx)
        .filter(Post::status.eq(PostStatus::Published))
        .all()
        .await
        .unwrap();

    assert_eq!(published.len(), 0);
}
