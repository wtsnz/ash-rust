#[path = "support/client_reads.rs"]
mod support;

use ash_core::{
    ClientExpr, DataLayer, FieldMap, Filter, FilterMatch, ReadRequest, Resource, Runtime, Value,
};
use support::*;

async fn relationship_conditions<D: DataLayer>(runtime: Runtime<D>) {
    seed(&runtime).await;
    for (actor, expected) in [
        (admin(), vec!["private", "public"]),
        (member(), vec!["public"]),
    ] {
        let ctx = reader(&runtime, actor);
        static TRUSTED_READS: &[ash_core::ActionDef] = &[ash_core::ActionDef::read("linked")
            .primary()
            .preparations(&[ash_core::PreparationDef::Filter(|| {
                ash_core::TrustedExpr::Filter(Filter::related("tags", Filter::True))
            })])];
        let trusted_resource = ash_core::ResourceDef {
            actions: TRUSTED_READS,
            ..Post::DEF
        };
        let trusted = ctx
            .access()
            .read(
                &trusted_resource,
                "linked",
                &FieldMap::new(),
                ReadRequest::default(),
            )
            .await
            .unwrap();
        let mut names: Vec<_> = trusted
            .iter()
            .map(|row| row.get("title").unwrap().as_str().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["private", "public"],
            "trusted preparations may depend on private related rows"
        );
        let found = Post::query(&ctx)
            .filter(ClientExpr::new(Filter::related("tags", Filter::True)).into_filter())
            .sort(Post::title)
            .all()
            .await
            .unwrap();
        assert_eq!(
            found
                .iter()
                .map(|row| row.title.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        let posts = Post::query(&ctx)
            .load_rel(post::fields::tags)
            .load_aggregate(post::fields::tag_count)
            .load_aggregate(post::fields::has_tags)
            .load_aggregate(post::fields::first_tag)
            .all()
            .await
            .unwrap();
        for post in posts {
            let readable = expected.contains(&post.title.as_str());
            assert_eq!(post.tag_count, Some(i64::from(readable)));
            assert_eq!(post.has_tags, Some(readable));
            assert_eq!(
                post.first_tag.as_deref(),
                readable.then_some(post.title.as_str())
            );
            assert_eq!(post.tags.as_slice().unwrap().len(), usize::from(readable));
        }
    }
}

#[tokio::test]
async fn memory_client_traversals_and_loads_scope_destination_and_join() {
    relationship_conditions(Runtime::new(ash_memory::Memory::new())).await;
}

#[tokio::test]
async fn sqlite_client_traversals_and_loads_scope_destination_and_join() {
    let sqlite = ash_sqlite::Sqlite::memory().await.unwrap();
    sqlite.install(RESOURCES).await.unwrap();
    relationship_conditions(Runtime::new(sqlite)).await;
}

async fn hidden_order<D: DataLayer>(runtime: Runtime<D>) {
    seed(&runtime).await;
    let ctx = reader(&runtime, member());
    let first = Payroll::query(&ctx)
        .page_keyset(1, None, None)
        .await
        .unwrap();
    assert_eq!(first.results[0].name, "high");
    assert_eq!(first.results[0].salary, None);
    let next = Payroll::query(&ctx)
        .page_keyset(1, first.after(), None)
        .await
        .unwrap();
    assert_eq!(next.results[0].name, "middle");
    assert_eq!(next.results[0].salary, None);
    let previous = Payroll::query(&ctx)
        .page_keyset(1, None, next.before())
        .await
        .unwrap();
    assert_eq!(previous.results[0].name, "high");
    let client = ctx
        .access()
        .read(
            &Payroll::DEF,
            "read",
            &FieldMap::new(),
            ReadRequest {
                filter: Some(ClientExpr::new(Filter::gte("salary", 100)).into_filter()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(client.is_empty());
    let negated = Payroll::query(&ctx)
        .filter(!ClientExpr::new(Filter::gte("salary", 100)).into_filter())
        .all()
        .await
        .unwrap();
    assert!(negated.is_empty());
    for (filter, expected) in [
        (Filter::eq("salary", Value::Null), 3),
        (!Filter::eq("salary", Value::Null), 0),
        (Filter::ne("salary", Value::Null), 0),
        (!Filter::ne("salary", Value::Null), 3),
        (!Filter::In("salary".into(), vec![]), 3),
    ] {
        let rows = ctx
            .access()
            .read(
                &Payroll::DEF,
                "read",
                &FieldMap::new(),
                ReadRequest {
                    filter: Some(ClientExpr::new(filter).into_filter()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), expected);
    }
    let arguments = [("salary".into(), Value::Int(200))].into_iter().collect();
    assert!(
        ctx.access()
            .read(
                &Payroll::DEF,
                "by_salary",
                &arguments,
                ReadRequest::default()
            )
            .await
            .unwrap()
            .is_empty()
    );
    let admin = reader(&runtime, admin());
    assert_eq!(
        admin
            .access()
            .read(
                &Payroll::DEF,
                "by_salary",
                &arguments,
                ReadRequest::default()
            )
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn memory_sealed_pages_preserve_hidden_prepared_sorts_and_client_arguments() {
    hidden_order(Runtime::new(ash_memory::Memory::new())).await;
}

#[tokio::test]
async fn sqlite_sealed_pages_preserve_hidden_prepared_sorts_and_client_arguments() {
    let sqlite = ash_sqlite::Sqlite::memory().await.unwrap();
    sqlite.install(RESOURCES).await.unwrap();
    hidden_order(Runtime::new(sqlite)).await;
}

#[test]
fn unloaded_relationship_snapshots_need_data_and_never_match() {
    let filter = Filter::related("tags", Filter::eq("name", "private"));
    let fields = FieldMap::new();
    assert_eq!(
        filter.snapshot_match(&Post::DEF, &fields),
        FilterMatch::NeedsData
    );
    assert!(!filter.matches_on(&Post::DEF, &fields));
    assert!(!(!filter.clone()).matches_on(&Post::DEF, &fields));
    assert!((!filter.clone()).snapshot_match(&Post::DEF, &fields) == FilterMatch::NeedsData);
    assert!(filter.validate_snapshot(&Post::DEF).is_err());
    assert!(
        Filter::gt("doubled", 0)
            .validate_snapshot(&Payroll::DEF)
            .is_err()
    );
    assert_eq!(
        Filter::True.snapshot_match(&Post::DEF, &fields),
        FilterMatch::Match
    );
    assert_eq!(
        Filter::False.snapshot_match(&Post::DEF, &fields),
        FilterMatch::NoMatch
    );
}

#[test]
fn cursor_encryption_rejects_forgery_replay_and_key_rotation() {
    use ash_core::{Actor, KeysetCursor, Sort};
    let runtime = Runtime::new(()).with_cursor_key([7; 32]);
    let ctx = reader(&runtime, member());
    let sort = vec![Sort {
        field: "salary".into(),
        descending: true,
        ..Default::default()
    }];
    let fields = [
        ("id".into(), Value::Uuid(uuid::Uuid::nil())),
        ("salary".into(), Value::Int(300)),
    ]
    .into_iter()
    .collect();
    let token = ctx
        .seal_keyset(&Payroll::DEF, "read", &sort, &fields)
        .unwrap();
    assert!(KeysetCursor::decode(&token).is_none());
    assert!(
        !ctx.keyset_filter(&Payroll::DEF, "read", &sort, &token, true)
            .unwrap()
            .matches_on(&Payroll::DEF, &fields)
    );
    assert_ne!(
        token,
        ctx.seal_keyset(&Payroll::DEF, "read", &sort, &fields)
            .unwrap()
    );
    assert!(!format!("{runtime:?} {ctx:?}").contains("[7, 7"));
    for index in [0, 4, 15, token.len() - 3] {
        let mut bytes = token.as_bytes().to_vec();
        bytes[index] = if bytes[index] == b'A' { b'B' } else { b'A' };
        assert!(
            ctx.keyset_filter(
                &Payroll::DEF,
                "read",
                &sort,
                std::str::from_utf8(&bytes).unwrap(),
                true
            )
            .is_err()
        );
    }
    let unsealed = KeysetCursor {
        id: Value::Uuid(uuid::Uuid::nil()),
        values: vec![("salary".into(), Value::Int(250))],
    };
    for forged in [
        unsealed.encode(),
        serde_json::to_string(&unsealed).unwrap(),
        uuid::Uuid::nil().to_string(),
    ] {
        assert!(
            ctx.keyset_filter(&Payroll::DEF, "read", &sort, &forged, true)
                .is_err()
        );
    }
    assert!(
        reader(&runtime, admin())
            .keyset_filter(&Payroll::DEF, "read", &sort, &token, true)
            .is_err()
    );
    assert!(
        runtime
            .bind(Some(member()), Some("elsewhere".into()))
            .keyset_filter(&Payroll::DEF, "read", &sort, &token, true)
            .is_err()
    );
    assert!(
        runtime
            .system()
            .bind(Some(member()), None)
            .keyset_filter(&Payroll::DEF, "read", &sort, &token, true)
            .is_err()
    );
    assert!(
        ctx.keyset_filter(&Post::DEF, "read", &sort, &token, true)
            .is_err()
    );
    assert!(
        ctx.keyset_filter(&Payroll::DEF, "other", &sort, &token, true)
            .is_err()
    );
    assert!(
        ctx.keyset_filter(&Payroll::DEF, "read", &[sort[0].reversed()], &token, true)
            .is_err()
    );
    let guarded = ash_core::plan_client_sort(
        &Payroll::DEF,
        Some(&member()),
        ClientExpr::new(sort.clone()),
    )
    .unwrap();
    assert!(
        ctx.keyset_filter(&Payroll::DEF, "read", &guarded, &token, true)
            .is_err()
    );
    let changed = Actor::new(member().id).with_role("admin");
    assert!(
        reader(&runtime, changed)
            .keyset_filter(&Payroll::DEF, "read", &sort, &token, true)
            .is_err()
    );
    let same_key = Runtime::new(()).with_cursor_key([7; 32]);
    assert!(
        reader(&same_key, member())
            .keyset_filter(&Payroll::DEF, "read", &sort, &token, true)
            .is_ok()
    );
    let rotated = Runtime::new(()).with_cursor_key([8; 32]);
    assert!(
        reader(&rotated, member())
            .keyset_filter(&Payroll::DEF, "read", &sort, &token, true)
            .is_err()
    );
    assert!(
        reader(&Runtime::new(()), member())
            .keyset_filter(&Payroll::DEF, "read", &sort, &token, true)
            .is_err()
    );
}

#[test]
fn a_guarded_identity_sort_still_needs_a_primary_key_tiebreaker() {
    let sort = ash_core::plan_client_sort(
        &Payroll::DEF,
        Some(&member()),
        ClientExpr::new(vec![ash_core::Sort {
            field: "lookup".into(),
            ..Default::default()
        }]),
    )
    .unwrap();
    let sort = ash_core::keyset_sort(&Payroll::DEF, sort);
    assert_eq!(
        sort.iter()
            .map(|sort| sort.field.as_str())
            .collect::<Vec<_>>(),
        ["lookup", "id"]
    );
}

#[test]
fn repeated_sort_fields_keep_their_distinct_guarded_cursor_values() {
    let sort = vec![
        ash_core::Sort {
            field: "salary".into(),
            ..Default::default()
        },
        ash_core::Sort {
            field: "salary".into(),
            guard: Some(Filter::False),
            ..Default::default()
        },
    ];
    let cursor = ash_core::KeysetCursor {
        id: Value::Uuid(uuid::Uuid::nil()),
        values: vec![
            ("salary".into(), Value::Int(300)),
            ("salary".into(), Value::Null),
        ],
    };
    assert_eq!(
        ash_core::keyset_values(&Payroll::DEF, &cursor, &sort),
        [Value::Int(300), Value::Null]
    );
}
