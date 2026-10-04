//! A generic action's own `run`, as Ash's: in a transaction when it takes one
//! (`transaction;`, Ash's `transaction? true`), and runnable by name with its input as
//! values (`Resource::run_generic`), as an API serving actions by name runs it.

use ash_core::{Actor, Context, Error, FieldMap, Resource, Value, resource};
use ash_memory::Memory;
use uuid::Uuid;

#[derive(Clone, Debug, serde::Serialize)]
pub struct Summary {
    pub count: usize,
}

resource! {
    Entry {
        table "generic_entries";

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            title: String;
        }

        policies {
            policy always {
                authorize_if actor_attribute_equals(role, "writer");
            }
        }

        actions {
            create create { primary; accept [title]; }
            read read { primary; }

            // Writes two entries, then fails if told to: in a transaction, neither stays.
            generic pair {
                transaction;
                argument fail: bool [default: false];
                returns Uuid;
                run |input| async move {
                    let first = Entry::create(input.ctx).title("first").await?;
                    Entry::create(input.ctx).title("second").await?;
                    if input.fail {
                        return Err(Error::Invalid("told to fail".into()));
                    }
                    Ok(first.id)
                };
            }

            generic shout {
                argument word: String;
                returns String;
                run |input| async move { Ok(input.word.to_uppercase()) };
            }

            generic summarize {
                returns Summary;
                run |input| async move {
                    let count = Entry::query(input.ctx).all().await?.len();
                    Ok(Summary { count })
                };
            }

            generic noop {
                run |_input| async move { Ok(()) };
            }
        }
    }
}

fn writer() -> Actor {
    Actor::new(Uuid::new_v4()).with_attr("role", Value::from("writer"))
}

#[tokio::test]
async fn a_generic_action_runs_in_its_transaction() {
    let ctx = Context::new(Memory::new()).with_actor(writer());
    let failed = Entry::pair(&ctx).fail(true).call().await;
    assert!(matches!(failed, Err(Error::Invalid(_))), "{failed:?}");
    assert_eq!(Entry::query(&ctx).count().await.unwrap(), 0, "rolled back");
    Entry::pair(&ctx).call().await.unwrap();
    assert_eq!(Entry::query(&ctx).count().await.unwrap(), 2);
}

#[tokio::test]
async fn generic_actions_run_by_name() {
    let ctx = Context::new(Memory::new()).with_actor(writer());
    let mut input = FieldMap::new();
    input.insert("word".into(), Value::from("hi"));
    assert_eq!(Entry::run_generic(&ctx, "shout", input).await.unwrap(), Value::from("HI"));
    // Defaults fill in, and the result is the action's own.
    let id = Entry::run_generic(&ctx, "pair", FieldMap::new()).await.unwrap();
    assert!(id.as_uuid().is_some(), "{id:?}");
    // A serializable result is what it serializes to; nothing returned is nil.
    let summary = Entry::run_generic(&ctx, "summarize", FieldMap::new()).await.unwrap();
    assert_eq!(summary.to_plain_json(), serde_json::json!({ "count": 2 }));
    assert_eq!(Entry::run_generic(&ctx, "noop", FieldMap::new()).await.unwrap(), Value::Null);
    // Missing arguments, and the action's policies, as a run of it.
    let missing = Entry::run_generic(&ctx, "shout", FieldMap::new()).await;
    assert!(matches!(missing, Err(Error::Missing { ref field }) if field == "word"), "{missing:?}");
    let stranger = ctx.with_actor(Actor::new(Uuid::new_v4()).with_attr("role", Value::from("reader")));
    let forbidden = Entry::run_generic(&stranger, "noop", FieldMap::new()).await;
    assert!(matches!(forbidden, Err(Error::Forbidden)), "{forbidden:?}");
}
