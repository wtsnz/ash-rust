use ash_core::{
    Actor, AttrType, Context, DataLayer, DataLayerKind, FieldMap, FragmentPart as P, Resource,
    Runtime, SqlFragment, TrustedExpr, resource,
};
use uuid::Uuid;

pub const PORTABLE: &[DataLayerKind] = &[DataLayerKind::Postgres, DataLayerKind::Sqlite];
pub const TITLE_MATCH: SqlFragment = SqlFragment::new(
    PORTABLE,
    &[
        P::Sql("lower("),
        P::Field("title"),
        P::Sql(") = lower("),
        P::Param {
            name: "term",
            ty: AttrType::String,
        },
        P::Sql(")"),
    ],
);
pub const SECRET_LENGTH: SqlFragment = SqlFragment::new(
    PORTABLE,
    &[P::Sql("length("), P::Field("secret"), P::Sql(")")],
);
pub const SEARCH: SqlFragment = SqlFragment::new(
    &[DataLayerKind::Postgres],
    &[
        P::Sql("to_tsvector('english', "),
        P::Field("title"),
        P::Sql(") @@ plainto_tsquery('english', "),
        P::Param {
            name: "term",
            ty: AttrType::String,
        },
        P::Sql(")"),
    ],
);
pub const RANK: SqlFragment = SqlFragment::new(
    &[DataLayerKind::Postgres],
    &[
        P::Sql("CAST(1000000 * ts_rank(to_tsvector('english', "),
        P::Field("title"),
        P::Sql("), plainto_tsquery('english', "),
        P::Param {
            name: "term",
            ty: AttrType::String,
        },
        P::Sql(")) AS bigint)"),
    ],
);
pub fn title_filter(args: &FieldMap) -> TrustedExpr {
    TITLE_MATCH.bind(args.clone())
}
pub fn search_filter(args: &FieldMap) -> TrustedExpr {
    SEARCH.bind(args.clone())
}

resource! {
    Document {
        table "trusted_node_documents";
        actor { role: String; }
        attributes {
            id: Uuid [pk]; tenant: String; title: String; category: String;
            secret: Option<String>; enabled: bool;
        }
        calculations {
            secret_length: Option<i64> = fragment(SECRET_LENGTH);
            public_length: Option<i64> [declassify] = fragment(SECRET_LENGTH);
            relevance(term: String): Option<i64> = fragment(RANK);
        }
        multitenancy { strategy attribute; attribute tenant; }
        policies {
            policy action_type(read) { authorize_if eq(enabled, true); }
            policy action_type(create) { authorize_if always; }
        }
        field_policies {
            field secret { authorize_if actor_attribute_equals(role, "admin"); }
            field relevance { authorize_if actor_attribute_equals(role, "admin"); }
        }
        actions {
            create create { primary; accept [id, title, category, secret, enabled]; }
            read read { primary; }
            read ranked { prepare sort(public_length, desc); }
            read matching {
                argument term: String;
                prepare trusted_filter(title_filter);
                prepare sort(public_length, desc);
            }
            read search {
                argument term: String;
                prepare trusted_filter(search_filter);
                prepare sort(relevance, desc);
            }
        }
    }
}

pub const RESOURCES: &[&ash_core::ResourceDef] = &[&Document::DEF];
pub fn reader<D>(runtime: &Runtime<D>) -> Context<D> {
    runtime.bind(
        Some(Actor::new(Uuid::nil()).with_role("member")),
        Some("a".into()),
    )
}

pub async fn seed<D: DataLayer>(runtime: &Runtime<D>) {
    for (id, tenant, title, category, secret, enabled) in [
        (1, "a", "Rust rust rust", "a", "long-secret", true),
        (2, "a", "Rust", "a", "short", true),
        (3, "a", "Rust systems", "b", "middle", true),
        (4, "a", "Rust hidden", "c", "hidden", false),
        (5, "b", "Only B", "b", "other", true),
        (6, "a", "quote ' OR 1=1 --", "d", "q", true),
    ] {
        let ctx = runtime.bind(None, Some(tenant.into()));
        Document::create(&ctx)
            .id(Uuid::from_u128(id))
            .title(title)
            .category(category)
            .secret(secret)
            .enabled(enabled)
            .await
            .unwrap();
    }
}

pub async fn fragment_results<D: DataLayer>(runtime: Runtime<D>) {
    seed(&runtime).await;
    let ctx = reader(&runtime);
    let rows = Document::query(&ctx)
        .action("matching")
        .argument("term", "rust")
        .calc(Document::secret_length)
        .calc(Document::public_length)
        .all()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "Rust");
    assert_eq!(rows[0].secret, None);
    assert_eq!(rows[0].secret_length, None);
    assert_eq!(rows[0].public_length, Some(5));
    for term in ["Only B", "Rust hidden"] {
        assert!(
            Document::query(&ctx)
                .action("matching")
                .argument("term", term)
                .all()
                .await
                .unwrap()
                .is_empty()
        );
    }
    let rows = Document::query(&ctx)
        .action("matching")
        .argument("term", "quote ' OR 1=1 --")
        .all()
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].category, "d");
    let rows = Document::query(&ctx)
        .action("ranked")
        .calc(Document::public_length)
        .all()
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r.public_length.unwrap())
            .collect::<Vec<_>>(),
        [11, 6, 5, 1]
    );
}
