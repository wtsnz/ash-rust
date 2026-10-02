//! Updates and destroys as one statement, as Ash runs them atomically, in Postgres and in
//! memory: what they set, computed from the record as stored; their validations, policies
//! and lock version checked in the statement; and the errors they fail with.

use ash_core::{
    ActionDef, ActionKind, Actor, Atomic, AtomicCondition, AtomicContext, AtomicExpr, AttrType,
    AttributeDef, Change, ChangeContext, Check, Context, CustomChange, CustomValidation, DataLayer,
    Error, FieldMap, PolicyDef, PolicyEffect, PolicyWhen, ResourceDef, Result, Validation,
    ValidationContext, Value, destroy_dynamic_by_id, update_dynamic, update_dynamic_expecting,
    update_existing_dynamic,
};
use ash_memory::Memory;
use ash_postgres::Postgres;
use uuid::Uuid;

/// `count + 1`, from the count as stored.
struct Increment;

impl CustomChange for Increment {
    fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()> {
        let count = ctx.fields.get("count").and_then(Value::as_int).unwrap_or(0);
        ctx.fields.insert("count".into(), Value::Int(count + 1));
        Ok(())
    }

    fn atomic(&self, _ctx: &AtomicContext<'_>) -> Atomic {
        Atomic::Atomic {
            set: vec![(
                "count".into(),
                AtomicExpr::Add(Box::new(AtomicExpr::field("count")), Box::new(AtomicExpr::value(1i64))),
            )],
            conditions: Vec::new(),
        }
    }
}

/// Only an open counter closes; the error says what it was.
struct MustBeOpen;

impl CustomValidation for MustBeOpen {
    fn validate(&self, ctx: &ValidationContext<'_>) -> Result<()> {
        let status = ctx.record.and_then(|r| r.get("status")).and_then(Value::as_str).unwrap_or("");
        if status == "open" { Ok(()) } else { Err(closed_error(status)) }
    }

    fn atomic(&self, _ctx: &AtomicContext<'_>) -> Atomic {
        Atomic::conditions(vec![AtomicCondition::new(
            AtomicExpr::DistinctFrom(Box::new(AtomicExpr::field("status")), Box::new(AtomicExpr::value("open"))),
            vec!["status".into()],
            |row| closed_error(row.get("status").and_then(Value::as_str).unwrap_or("")),
        )])
    }
}

fn closed_error(status: &str) -> Error {
    Error::Validation {
        field: "status".into(),
        message: format!("is {status}, not open"),
    }
}

static INCREMENT: Increment = Increment;
static MUST_BE_OPEN: MustBeOpen = MustBeOpen;

fn shout(ctx: &mut ChangeContext<'_>) -> Result<()> {
    let name = ctx.fields.get("name").and_then(Value::as_str).unwrap_or("").to_uppercase();
    ctx.fields.insert("name".into(), Value::String(name));
    Ok(())
}

static ACTIONS: &[ActionDef] = &[
    ActionDef::create("create").accept(&["name", "status", "count", "owner_id"]),
    ActionDef::read("read").primary(),
    ActionDef::update("rename")
        .accept(&["name"])
        .validations(&[Validation::string_length("name", Some(2), None), Validation::present("status")]),
    ActionDef::update("bump").changes(&[Change::Custom(&INCREMENT)]),
    ActionDef::update("close")
        .changes(&[Change::SetAttribute { field: "status", value: ash_core::ConstValue::Str("closed") }])
        .validations(&[Validation::Custom(&MUST_BE_OPEN)]),
    ActionDef::update("shout").changes(&[Change::Func(shout)]),
    ActionDef::update("shout_reading_first").changes(&[Change::Func(shout)]).require_atomic(false),
    // Only an open counter goes.
    ActionDef::destroy("remove").validations(&[Validation::Custom(&MUST_BE_OPEN)]),
    ActionDef::destroy("shout_and_remove").changes(&[Change::Func(shout)]),
    ActionDef::destroy("archive")
        .soft()
        .changes(&[Change::SetAttributeFn { field: "archived_at", value: now }]),
    ActionDef::destroy("shout_and_archive").soft().changes(&[Change::Func(shout)]),
    ActionDef::destroy("shout_and_archive_reading_first")
        .soft()
        .changes(&[Change::Func(shout)])
        .require_atomic(false),
];

fn now() -> Value {
    ash_core::AshType::to_value(&ash_core::UtcDateTimeUsec::now())
}

static POLICIES: &[PolicyDef] = &[
    PolicyDef::when(PolicyWhen::ActionType(ActionKind::Create), &[PolicyEffect::AuthorizeIf(Check::Always)]),
    PolicyDef::when(PolicyWhen::ActionType(ActionKind::Read), &[PolicyEffect::AuthorizeIf(Check::Always)]),
    // Only its owner updates a counter, or destroys it.
    PolicyDef::when(
        PolicyWhen::ActionType(ActionKind::Update),
        &[PolicyEffect::AuthorizeIf(Check::RelatesToActor { field: "owner_id" })],
    ),
    PolicyDef::when(
        PolicyWhen::ActionType(ActionKind::Destroy),
        &[PolicyEffect::AuthorizeIf(Check::RelatesToActor { field: "owner_id" })],
    ),
];

static COUNTER: ResourceDef = ResourceDef {
    name: "AtomicCounter",
    table: "atomic_counters",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("name", AttrType::String),
        AttributeDef::required("status", AttrType::String),
        AttributeDef::required("count", AttrType::Integer),
        AttributeDef::optional("owner_id", AttrType::Uuid),
        AttributeDef::version("version"),
        AttributeDef::optional("created_at", AttrType::UTC_DATETIME_USEC),
        AttributeDef::optional("updated_at", AttrType::UTC_DATETIME_USEC),
        AttributeDef::optional("archived_at", AttrType::UTC_DATETIME_USEC),
    ],
    relationships: &[],
    actions: ACTIONS,
    policies: POLICIES,
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[],
    indexes: &[],
    checks: &[],
    statements: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Postgres,
    timestamps: Some(("created_at", "updated_at")),
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

fn action(name: &str) -> &'static ActionDef {
    COUNTER.action(name).expect("an action")
}

fn input(pairs: &[(&str, Value)]) -> FieldMap {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

/// A counter owned by `owner`, open, at zero.
async fn counter<D: DataLayer>(ctx: &Context<D>, owner: Uuid) -> FieldMap {
    ash_core::create_dynamic(
        ctx,
        &COUNTER,
        action("create"),
        input(&[
            ("name", Value::from("first")),
            ("status", Value::from("open")),
            ("count", Value::Int(0)),
            ("owner_id", Value::Uuid(owner)),
        ]),
    )
    .await
    .unwrap()
}

fn id_of(record: &FieldMap) -> Uuid {
    record.get("id").and_then(Value::as_uuid).unwrap()
}

async fn scenario<D: DataLayer + Clone + 'static>(data: D) {
    assert!(data.can_update_atomically(&COUNTER));
    let owner = Uuid::new_v4();
    let ctx = Context::new(data).with_actor(Actor::new(owner));
    let created = counter(&ctx, owner).await;
    let id = id_of(&created);

    // What it sets, with the version bumped and `updated_at` moved.
    let renamed = update_dynamic(&ctx, &COUNTER, action("rename"), id, input(&[("name", Value::from("second"))]))
        .await
        .unwrap();
    assert_eq!(renamed.get("name"), Some(&Value::from("second")));
    assert_eq!(renamed.get("version"), Some(&Value::Int(2)));
    assert_ne!(renamed.get("updated_at"), created.get("updated_at"));

    // Setting what it holds changes nothing, so `updated_at` stays, as in Ash.
    let same = update_dynamic(&ctx, &COUNTER, action("rename"), id, input(&[("name", Value::from("second"))]))
        .await
        .unwrap();
    assert_eq!(same.get("updated_at"), renamed.get("updated_at"));
    assert_eq!(same.get("version"), Some(&Value::Int(3)));

    // A value it sets that fails validation fails before the statement.
    let short = update_dynamic(&ctx, &COUNTER, action("rename"), id, input(&[("name", Value::from("x"))])).await;
    assert!(matches!(short, Err(Error::Validation { ref field, .. }) if field == "name"), "{short:?}");

    // A condition on the stored record fails in the statement, with the error it gives.
    let closed = update_dynamic(&ctx, &COUNTER, action("close"), id, FieldMap::new()).await.unwrap();
    assert_eq!(closed.get("status"), Some(&Value::from("closed")));
    let again = update_dynamic(&ctx, &COUNTER, action("close"), id, FieldMap::new()).await;
    match again {
        Err(Error::Validation { field, message }) => {
            assert_eq!(field, "status");
            assert_eq!(message, "is closed, not open");
        }
        other => panic!("expected the closed error, got {other:?}"),
    }

    // Policies are checked against the stored record: only the owner updates it.
    let stranger = ctx.with_actor(Actor::new(Uuid::new_v4()));
    let forbidden = update_dynamic(&stranger, &COUNTER, action("bump"), id, FieldMap::new()).await;
    assert!(matches!(forbidden, Err(Error::Forbidden)), "{forbidden:?}");
    // A counter with no owner relates to no one: a policy that can't be met forbids.
    let ownerless = ash_core::create_dynamic(
        &ctx,
        &COUNTER,
        action("create"),
        input(&[("name", Value::from("nobody's")), ("status", Value::from("open")), ("count", Value::Int(0))]),
    )
    .await
    .unwrap();
    let forbidden = update_dynamic(&ctx, &COUNTER, action("bump"), id_of(&ownerless), FieldMap::new()).await;
    assert!(matches!(forbidden, Err(Error::Forbidden)), "{forbidden:?}");

    // No such record: not found. Another version: stale.
    let missing = update_dynamic(&ctx, &COUNTER, action("bump"), Uuid::new_v4(), FieldMap::new()).await;
    assert!(matches!(missing, Err(Error::NotFound)), "{missing:?}");
    let stale = update_dynamic_expecting(&ctx, &COUNTER, action("bump"), id, FieldMap::new(), Some(1)).await;
    assert!(matches!(stale, Err(Error::StaleRecord { .. })), "{stale:?}");

    // The record in hand, updated from a stale copy: stale.
    let stale = update_existing_dynamic(&ctx, &COUNTER, action("bump"), created.clone(), FieldMap::new()).await;
    assert!(matches!(stale, Err(Error::StaleRecord { .. })), "{stale:?}");

    // Increments from the stored count: none lost, however many at once.
    let fresh = counter(&ctx, owner).await;
    let fresh_id = id_of(&fresh);
    let bumps = (0..20).map(|_| {
        let ctx = ctx.clone();
        tokio::spawn(async move { update_dynamic(&ctx, &COUNTER, action("bump"), fresh_id, FieldMap::new()).await })
    });
    for bump in bumps {
        bump.await.unwrap().unwrap();
    }
    let counted = update_dynamic(&ctx, &COUNTER, action("rename"), fresh_id, input(&[("name", Value::from("done"))]))
        .await
        .unwrap();
    assert_eq!(counted.get("count"), Some(&Value::Int(20)));
    assert_eq!(counted.get("version"), Some(&Value::Int(22)));

    // A change that needs the record in memory: must be atomic, unless it reads first.
    let must = update_dynamic(&ctx, &COUNTER, action("shout"), fresh_id, FieldMap::new()).await;
    assert!(matches!(must, Err(Error::MustBeAtomic { action: "shout", .. })), "{must:?}");
    let shouted = update_dynamic(&ctx, &COUNTER, action("shout_reading_first"), fresh_id, FieldMap::new())
        .await
        .unwrap();
    assert_eq!(shouted.get("name"), Some(&Value::from("DONE")));
}

/// Destroys by id: a hard destroy as one delete, a soft one as one update.
async fn destroy_scenario<D: DataLayer + Clone + 'static>(data: D) {
    assert!(data.can_destroy_atomically(&COUNTER));
    let owner = Uuid::new_v4();
    let ctx = Context::new(data).with_actor(Actor::new(owner));
    let removable = counter(&ctx, owner).await;
    let id = id_of(&removable);

    // Policies are checked against the stored record: only the owner destroys it.
    let stranger = ctx.with_actor(Actor::new(Uuid::new_v4()));
    let forbidden = destroy_dynamic_by_id(&stranger, &COUNTER, action("remove"), id, None).await;
    assert!(matches!(forbidden, Err(Error::Forbidden)), "{forbidden:?}");
    // Another version: stale.
    let stale = destroy_dynamic_by_id(&ctx, &COUNTER, action("remove"), id, Some(2)).await;
    assert!(matches!(stale, Err(Error::StaleRecord { .. })), "{stale:?}");

    // It returns the record as it was deleted, and then there's none.
    let removed = destroy_dynamic_by_id(&ctx, &COUNTER, action("remove"), id, Some(1)).await.unwrap();
    assert_eq!(removed.get("name"), Some(&Value::from("first")));
    let missing = destroy_dynamic_by_id(&ctx, &COUNTER, action("remove"), id, None).await;
    assert!(matches!(missing, Err(Error::NotFound)), "{missing:?}");

    // A condition on the stored record fails in the statement, and nothing goes.
    let closing = counter(&ctx, owner).await;
    let closing_id = id_of(&closing);
    update_dynamic(&ctx, &COUNTER, action("close"), closing_id, FieldMap::new()).await.unwrap();
    let refused = destroy_dynamic_by_id(&ctx, &COUNTER, action("remove"), closing_id, None).await;
    match refused {
        Err(Error::Validation { message, .. }) => assert_eq!(message, "is closed, not open"),
        other => panic!("expected the closed error, got {other:?}"),
    }
    let still_there = update_dynamic(&ctx, &COUNTER, action("bump"), closing_id, FieldMap::new()).await;
    assert!(still_there.is_ok(), "{still_there:?}");

    // A hard destroy that can't be one statement reads first, as in Ash.
    let shouted = destroy_dynamic_by_id(&ctx, &COUNTER, action("shout_and_remove"), closing_id, None).await;
    assert!(shouted.is_ok(), "{shouted:?}");
    let gone = destroy_dynamic_by_id(&ctx, &COUNTER, action("remove"), closing_id, None).await;
    assert!(matches!(gone, Err(Error::NotFound)), "{gone:?}");

    // A soft destroy is an update: the version bumped, the change made.
    let archivable = counter(&ctx, owner).await;
    let archivable_id = id_of(&archivable);
    let archived = destroy_dynamic_by_id(&ctx, &COUNTER, action("archive"), archivable_id, Some(1)).await.unwrap();
    assert!(!archived.get("archived_at").is_none_or(Value::is_null), "{archived:?}");
    assert_eq!(archived.get("version"), Some(&Value::Int(2)));

    // A soft destroy that can't be one statement must be atomic, unless it reads first.
    let must = destroy_dynamic_by_id(&ctx, &COUNTER, action("shout_and_archive"), archivable_id, None).await;
    assert!(matches!(must, Err(Error::MustBeAtomic { action: "shout_and_archive", .. })), "{must:?}");
    let shouted = destroy_dynamic_by_id(&ctx, &COUNTER, action("shout_and_archive_reading_first"), archivable_id, None)
        .await
        .unwrap();
    assert_eq!(shouted.get("name"), Some(&Value::from("FIRST")));

    // A soft destroy of the record in hand runs as one update too: from a stale copy, stale.
    let stale = ash_core::DynamicChangeset::for_destroy(&ctx, &COUNTER, action("archive"), archivable)
        .unwrap()
        .commit(&ctx)
        .await;
    assert!(matches!(stale, Err(Error::StaleRecord { .. })), "{stale:?}");
}

#[tokio::test]
async fn updates_run_atomically_in_postgres() {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".into());
    let Ok(pg) = Postgres::connect(&url).await else {
        assert!(std::env::var("CI").is_err(), "CI runs Postgres");
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&COUNTER]).await.unwrap();
    scenario(pg).await;
}

#[tokio::test]
async fn updates_run_atomically_in_memory() {
    scenario(Memory::new()).await;
}

#[tokio::test]
async fn destroys_run_atomically_in_postgres() {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".into());
    let Ok(pg) = Postgres::connect(&url).await else {
        assert!(std::env::var("CI").is_err(), "CI runs Postgres");
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&COUNTER]).await.unwrap();
    destroy_scenario(pg).await;
}

#[tokio::test]
async fn destroys_run_atomically_in_memory() {
    destroy_scenario(Memory::new()).await;
}
