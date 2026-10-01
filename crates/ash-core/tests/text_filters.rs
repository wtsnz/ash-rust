use ash_core::{CiString, Context, DataLayer, Filter, Resource, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

resource! {
    Contact {
        table "contacts";

        attributes {
            id: Uuid [pk];
            name: String;
            email: Option<CiString>;
        }

        actions {
            create create {
                primary;
                accept [name, email];
            }

            read read {
                primary;
            }
        }
    }
}

async fn seed<D: DataLayer>(ctx: &Context<D>) {
    for (name, email) in [
        ("Ada Lovelace", "Ada@Example.com"),
        ("Grace 100%", "grace@example.com"),
        ("Linus_T", "linus@example.org"),
    ] {
        Contact::create(ctx)
            .name(name)
            .email(CiString::parse(email).unwrap())
            .await
            .unwrap();
    }
    Contact::create(ctx).name("No Email").await.unwrap();
}

async fn names<D: DataLayer>(ctx: &Context<D>, filter: Filter) -> Vec<String> {
    let mut names: Vec<String> = Contact::query(ctx)
        .filter(filter)
        .all()
        .await
        .unwrap()
        .into_iter()
        .map(|contact| contact.name)
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn text_filters_match_in_memory_and_sqlite() {
    let mem_ctx = Context::new(Memory::new());
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Contact::DEF]).await.unwrap();
    let sql_ctx = Context::new(sqlite);
    seed(&mem_ctx).await;
    seed(&sql_ctx).await;

    let cases: Vec<(Filter, Vec<&str>)> = vec![
        (Contact::name.contains("Love"), vec!["Ada Lovelace"]),
        (Contact::name.contains("love"), vec![]),
        (Contact::name.starts_with("Grace"), vec!["Grace 100%"]),
        (Contact::name.ends_with("%"), vec!["Grace 100%"]),
        (Contact::name.contains("_"), vec!["Linus_T"]),
        (Contact::email.contains("EXAMPLE.COM"), vec!["Ada Lovelace", "Grace 100%"]),
        (Contact::email.starts_with("ada@"), vec!["Ada Lovelace"]),
        (Contact::email.ends_with(".ORG"), vec!["Linus_T"]),
        // A null email makes the match unknown, and NOT of unknown is still unknown.
        (!Contact::email.contains("EXAMPLE.COM"), vec!["Linus_T"]),
        (!Contact::email.eq(CiString::parse("ada@example.com").unwrap()), vec!["Grace 100%", "Linus_T"]),
        (
            !Filter::or([Contact::email.ends_with(".org"), Filter::IsNil("email".into())]),
            vec!["Ada Lovelace", "Grace 100%"],
        ),
        (
            Filter::or([!Contact::email.contains("example"), Contact::name.contains("Email")]),
            vec!["No Email"],
        ),
        (
            !Filter::In("email".into(), vec!["linus@example.org".into(), ash_core::Value::Null]),
            vec![],
        ),
    ];
    for (filter, expected) in cases {
        assert_eq!(names(&mem_ctx, filter.clone()).await, expected, "memory: {filter:?}");
        assert_eq!(names(&sql_ctx, filter.clone()).await, expected, "sqlite: {filter:?}");
    }
}
