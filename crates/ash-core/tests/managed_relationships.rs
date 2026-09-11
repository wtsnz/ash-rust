use ash_core::{
    Context, Error, FieldMap, ManagedRelType, Resource, Result, Value, resource,
};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

pub mod line_item_mod {
    use super::*;

    resource! {
        LineItem {
        table "line_items";

        attributes {
            id: Uuid [pk];
            order_id: Uuid;
            item: String;
            price: i64;
        }

        relationships {
            belongs_to order: order_mod::Order [fk: order_id];
        }

        actions {
            create create {
                primary;
                accept [order_id, item, price];
                validate numericality(price, min: 0);
            }

            read read {
                primary;
            }

            update update {
                primary;
                accept [item, price];
                validate numericality(price, min: 0);
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod order_mod {
    use super::*;

    resource! {
        Order {
        table "orders";

        attributes {
            id: Uuid [pk];
            customer: String;
        }

        relationships {
            has_many items: line_item_mod::LineItem [fk: order_id];
        }

        actions {
            create create {
                primary;
                accept [customer];
            }

            create create_with_items {
                accept [customer];
                argument items: Vec<FieldMap>;
                change manage_relationship(items, create);
            }

            read read {
                primary;
            }

            update update {
                primary;
                accept [customer];
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod player_mod {
    use super::*;

    resource! {
        Player {
        table "players";

        attributes {
            id: Uuid [pk];
            team_id: Option<Uuid>;
            name: String;
        }

        relationships {
            belongs_to team: team_mod::Team [fk: team_id];
        }

        actions {
            create create {
                primary;
                accept [team_id, name];
            }

            read read {
                primary;
            }

            update update {
                primary;
                accept [team_id, name];
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod team_mod {
    use super::*;

    resource! {
        Team {
        table "teams";

        attributes {
            id: Uuid [pk];
            name: String;
        }

        relationships {
            has_many players: player_mod::Player [fk: team_id, on_delete: nilify];
        }

        actions {
            create create {
                primary;
                accept [name];
            }

            read read {
                primary;
            }

            update update {
                primary;
                accept [name];
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod profile_mod {
    use super::*;

    resource! {
        Profile {
        table "profiles";

        attributes {
            id: Uuid [pk];
            bio: String;
        }

        actions {
            create create {
                primary;
                accept [bio];
            }

            read read {
                primary;
            }

            update update {
                primary;
                accept [bio];
            }
        }
    }}
}

pub mod user_mod {
    use super::*;

    resource! {
        User {
        table "users";

        attributes {
            id: Uuid [pk];
            profile_id: Option<Uuid>;
            username: String;
        }

        relationships {
            belongs_to profile: profile_mod::Profile [fk: profile_id];
        }

        actions {
            create create {
                primary;
                accept [profile_id, username];
            }

            read read {
                primary;
            }

            update update {
                primary;
                accept [username];
            }
        }
    }}
}

pub mod tag_mod {
    use super::*;

    resource! {
        Tag {
        table "tags";

        attributes {
            id: Uuid [pk];
            name: String;
        }

        actions {
            create create {
                primary;
                accept [name];
            }

            read read {
                primary;
            }
        }
    }}
}

pub mod article_tag_mod {
    use super::*;

    resource! {
        ArticleTag {
        table "article_tags";

        attributes {
            id: Uuid [pk];
            article_id: Uuid;
            tag_id: Uuid;
        }

        actions {
            create create {
                primary;
                accept [article_id, tag_id];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod article_mod {
    use super::*;
    use super::article_tag_mod::ArticleTag;
    use super::tag_mod::Tag;

    resource! {
        Article {
        table "articles";

        attributes {
            id: Uuid [pk];
            title: String;
        }

        relationships {
            many_to_many tags: Tag [
                through: ArticleTag,
                source_fk: article_id,
                dest_fk: tag_id,
            ];
        }

        actions {
            create create {
                primary;
                accept [title];
            }

            read read {
                primary;
            }

            update update {
                primary;
                accept [title];
            }
        }
    }}
}

use article_mod::Article;
use article_tag_mod::ArticleTag;
use line_item_mod::LineItem;
use order_mod::Order;
use player_mod::Player;
use profile_mod::Profile;
use tag_mod::Tag;
use team_mod::Team;
use user_mod::User;

#[tokio::test]
async fn test_managed_has_many_create_on_parent_create() -> Result<()> {
    let ctx = Context::new(Memory::new());

    // Create an Order and manage LineItem creation inside the same action
    let order = Order::create(&ctx)
        .customer("Alice")
        .manage_relationship(
            "items",
            vec![
                LineItem::create(&ctx).item("Book").price(25),
                LineItem::create(&ctx).item("Pen").price(5),
            ],
            ManagedRelType::Create,
        )
        .await?;

    assert_eq!(order.customer, "Alice");

    // Verify children in data layer have parent order_id
    let items = LineItem::query(&ctx)
        .filter(LineItem::order_id.eq(order.id))
        .all()
        .await?;
    assert_eq!(items.len(), 2);

    let mut item_names: Vec<String> = items.into_iter().map(|i| i.item).collect();
    item_names.sort();
    assert_eq!(item_names, vec!["Book", "Pen"]);

    Ok(())
}

#[tokio::test]
async fn test_managed_has_many_direct_control_on_parent_update() -> Result<()> {
    let ctx = Context::new(Memory::new());

    // 1. Initial order with 2 items: item1 and item2
    let order = Order::create(&ctx).customer("Bob").await?;
    let item1 = LineItem::create(&ctx).order_id(order.id).item("Monitor").price(300).await?;
    let item2 = LineItem::create(&ctx).order_id(order.id).item("Mouse").price(50).await?;

    let items_before = LineItem::query(&ctx).filter(LineItem::order_id.eq(order.id)).all().await?;
    assert_eq!(items_before.len(), 2);

    // 2. Synchronize via DirectControl on update:
    // - Keep and update item1 (change price to 280)
    // - Add new item3 ("Keyboard", price 100)
    // - Omit item2 ("Mouse") -> DirectControl should destroy it
    let mut updated_item1 = FieldMap::new();
    updated_item1.insert("id".into(), Value::Uuid(item1.id));
    updated_item1.insert("price".into(), Value::Int(280));

    let updated_order = Order::update(&ctx, order.id)
        .customer("Bob Updated")
        .manage_items(
            vec![
                updated_item1,
                LineItem::create(&ctx).item("Keyboard").price(100).into_fields(),
            ],
            ManagedRelType::DirectControl,
        )
        .await?;

    assert_eq!(updated_order.customer, "Bob Updated");

    let items_after = LineItem::query(&ctx).filter(LineItem::order_id.eq(updated_order.id)).all().await?;
    assert_eq!(items_after.len(), 2);

    // Check item1 price updated
    let refreshed_item1 = LineItem::get(&ctx, item1.id).await?;
    assert_eq!(refreshed_item1.price, 280);

    // Check item2 was removed
    assert!(LineItem::get(&ctx, item2.id).await.is_err());

    // Check new item was created
    let keyboard = items_after.into_iter().find(|i| i.item == "Keyboard").expect("keyboard created");
    assert_eq!(keyboard.price, 100);
    assert_eq!(keyboard.order_id, updated_order.id);

    Ok(())
}

#[tokio::test]
async fn test_managed_has_many_direct_control_nilify() -> Result<()> {
    let ctx = Context::new(Memory::new());

    let team = Team::create(&ctx).name("Lakers").await?;
    let p1 = Player::create(&ctx).team_id(Some(team.id)).name("LeBron").await?;
    let p2 = Player::create(&ctx).team_id(Some(team.id)).name("Davis").await?;

    // Update team with DirectControl, keeping only p1
    let mut keep_p1 = FieldMap::new();
    keep_p1.insert("id".into(), Value::Uuid(p1.id));
    keep_p1.insert("name".into(), Value::String("King James".into()));

    Team::update(&ctx, team.id)
        .manage_players(vec![keep_p1], ManagedRelType::DirectControl)
        .await?;

    // p1 updated and still on team
    let p1_after = Player::get(&ctx, p1.id).await?;
    assert_eq!(p1_after.name, "King James");
    assert_eq!(p1_after.team_id, Some(team.id));

    // p2 omitted, but relationship has on_delete: nilify -> player still exists, team_id is None!
    let p2_after = Player::get(&ctx, p2.id).await?;
    assert_eq!(p2_after.team_id, None);
    assert_eq!(p2_after.name, "Davis");

    Ok(())
}

#[tokio::test]
async fn test_managed_belongs_to_create() -> Result<()> {
    let ctx = Context::new(Memory::new());

    // When User is created, nested Profile is created first and its ID is linked to user.profile_id
    let user = User::create(&ctx)
        .username("will")
        .manage_relationship_one(
            "profile",
            Profile::create(&ctx).bio("Software Engineer"),
            ManagedRelType::Create,
        )
        .await?;

    assert!(user.profile_id.is_some());
    let profile_id = user.profile_id.unwrap();

    let profile = Profile::get(&ctx, profile_id).await?;
    assert_eq!(profile.bio, "Software Engineer");

    Ok(())
}

#[tokio::test]
async fn test_managed_many_to_many_create_and_direct_control() -> Result<()> {
    let ctx = Context::new(Memory::new());

    // Create article with two new tags
    let article = Article::create(&ctx)
        .title("Rust Metaprogramming")
        .manage_relationship(
            "tags",
            vec![
                Tag::create(&ctx).name("rust"),
                Tag::create(&ctx).name("macros"),
            ],
            ManagedRelType::Create,
        )
        .await?;

    // Verify join records were created
    let joins = ArticleTag::query(&ctx)
        .filter(ArticleTag::article_id.eq(article.id))
        .all()
        .await?;
    assert_eq!(joins.len(), 2);
    assert_eq!(Tag::query(&ctx).all().await?.len(), 2);

    // Synchronize tags using DirectControl: link to a new tag "ash", omit previous tags
    Article::update(&ctx, article.id)
        .manage_tags(
            vec![Tag::create(&ctx).name("ash")],
            ManagedRelType::DirectControl,
        )
        .await?;

    let joins_after = ArticleTag::query(&ctx)
        .filter(ArticleTag::article_id.eq(article.id))
        .all()
        .await?;
    assert_eq!(joins_after.len(), 1);

    Ok(())
}

#[tokio::test]
async fn test_action_dsl_change_manage_relationship() -> Result<()> {
    let ctx = Context::new(Memory::new());

    // Action declares:
    // create create_with_items {
    //     accept [customer];
    //     argument items: Vec<FieldMap>;
    //     change manage_relationship(items, type: create);
    // }
    let mut item1 = FieldMap::new();
    item1.insert("item".into(), Value::String("Coffee".into()));
    item1.insert("price".into(), Value::Int(4));

    let mut item2 = FieldMap::new();
    item2.insert("item".into(), Value::String("Croissant".into()));
    item2.insert("price".into(), Value::Int(5));

    let order = Order::create_with_items(&ctx)
        .customer("Carol")
        .items(vec![item1, item2])
        .await?;

    assert_eq!(order.customer, "Carol");

    let items = LineItem::query(&ctx)
        .filter(LineItem::order_id.eq(order.id))
        .all()
        .await?;
    assert_eq!(items.len(), 2);

    Ok(())
}

#[tokio::test]
async fn test_managed_relationship_atomic_rollback_on_child_error() -> Result<()> {
    let sqlite = Sqlite::memory().await?;
    let ctx = Context::new(sqlite);
    ctx.install(&[&Order::DEF, &LineItem::DEF]).await?;

    // Attempt to create Order with one valid item and one item that fails validation (price = -10)
    let res = Order::create(&ctx)
        .customer("Dave")
        .manage_relationship(
            "items",
            vec![
                LineItem::create(&ctx).item("Valid Item").price(20),
                LineItem::create(&ctx).item("Invalid Item").price(-10),
            ],
            ManagedRelType::Create,
        )
        .await;

    // Must return an error due to LineItem numericality validation
    assert!(res.is_err());
    match res {
        Err(Error::Validation { field, message }) => {
            assert_eq!(field, "price");
            assert!(message.contains("at least 0"));
        }
        other => panic!("expected Validation on child, got {other:?}"),
    }

    // Verify parent was NOT saved in SQLite because transaction was aborted
    assert!(Order::query(&ctx).all().await?.is_empty());
    assert!(LineItem::query(&ctx).all().await?.is_empty());

    Ok(())
}
