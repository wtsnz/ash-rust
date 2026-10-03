use ash_core::{AttrType, AttributeDef, Context, DataLayer, FieldMap, ResourceDef, Value};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;
use uuid::Uuid;

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required("priority", AttrType::Integer),
];

static TICKET_DEF: ResourceDef = ResourceDef {
    name: "Ticket",
    table: "tickets",
    attributes: TICKET_ATTRS,
    relationships: &[],
    actions: &[],
    policies: &[],
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
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

async fn seed_tickets(data: &Memory) {
    for i in 1..=5 {
        let id = Uuid::new_v4();
        let mut map = FieldMap::new();
        map.insert("id".into(), Value::Uuid(id));
        map.insert("title".into(), Value::String(format!("Ticket #{i}")));
        map.insert("priority".into(), Value::Int(i));
        data.create(&TICKET_DEF, None, Value::from(id), map).await.unwrap();
    }
}

/// Pages a list query through its keyset pages, as AshGraphql's keyset pagination does:
/// `first` / `after` forwards, `last` / `before` backwards.
#[tokio::test]
async fn list_queries_page_by_keyset() {
    let memory = Memory::new();
    seed_tickets(&memory).await;
    let ctx = Context::new(memory);
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");
    let run = |args: String| {
        let schema = schema.clone();
        let ctx = ctx.clone();
        async move {
            let query = format!(
                "{{ listTickets(sort: [{{ field: PRIORITY, order: ASC }}]{args}) {{ count startKeyset endKeyset results {{ priority }} }} }}"
            );
            let res = schema.execute(Request::new(query).data(ctx)).await;
            assert!(res.errors.is_empty(), "{:?}", res.errors);
            res.data.into_json().unwrap()["listTickets"].clone()
        }
    };
    let priorities = |page: &serde_json::Value| -> Vec<i64> {
        page["results"].as_array().unwrap().iter().map(|t| t["priority"].as_i64().unwrap()).collect()
    };

    // Without paging, every record.
    assert_eq!(priorities(&run(String::new()).await), [1, 2, 3, 4, 5]);

    let first = run(", first: 2".into()).await;
    assert_eq!(priorities(&first), [1, 2]);
    assert_eq!(first["count"], 5);
    let after = first["endKeyset"].as_str().unwrap().to_string();

    let second = run(format!(r#", first: 2, after: "{after}""#)).await;
    assert_eq!(priorities(&second), [3, 4]);
    let after = second["endKeyset"].as_str().unwrap().to_string();

    let third = run(format!(r#", first: 2, after: "{after}""#)).await;
    assert_eq!(priorities(&third), [5]);

    // Backwards from the third page's start.
    let before = third["startKeyset"].as_str().unwrap().to_string();
    let back = run(format!(r#", last: 2, before: "{before}""#)).await;
    assert_eq!(priorities(&back), [3, 4]);

    // A page holds at most 250 records, as Ash's default `max_page_size`.
    assert_eq!(priorities(&run(", first: 1000".into()).await).len(), 5);
}

/// Every list pages, as AshGraphql's do: given no `first` or `last`, a page holds as many
/// records as a page may, 250, and its end keyset reads on.
#[tokio::test]
async fn a_list_without_first_pages_as_many_as_a_page_may_hold() {
    let memory = Memory::new();
    for i in 0..260 {
        let id = Uuid::new_v4();
        let mut map = FieldMap::new();
        map.insert("id".into(), Value::Uuid(id));
        map.insert("title".into(), Value::String(format!("Ticket #{i}")));
        map.insert("priority".into(), Value::Int(i));
        memory.create(&TICKET_DEF, None, Value::from(id), map).await.unwrap();
    }
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");
    let ctx = Context::new(memory);
    let run = |query: String| {
        let (schema, ctx) = (schema.clone(), ctx.clone());
        async move {
            let res = schema.execute(Request::new(query).data(ctx)).await;
            assert!(res.errors.is_empty(), "{:?}", res.errors);
            res.data.into_json().unwrap()["listTickets"].clone()
        }
    };
    let page = run("{ listTickets(sort: [{ field: PRIORITY }]) { count results { priority } endKeyset } }".into()).await;
    assert_eq!(page["count"], 260);
    assert_eq!(page["results"].as_array().unwrap().len(), 250);
    let after = page["endKeyset"].as_str().expect("a keyset");
    let rest = run(format!(
        r#"{{ listTickets(sort: [{{ field: PRIORITY }}], after: "{after}") {{ results {{ priority }} }} }}"#
    ))
    .await;
    let rest: Vec<i64> = rest["results"].as_array().unwrap().iter().map(|t| t["priority"].as_i64().unwrap()).collect();
    assert_eq!(rest, (250..260).collect::<Vec<_>>());
}

/// The paging arguments AshGraphql refuses, refused as it refuses them.
#[tokio::test]
async fn paging_arguments_that_contradict_are_refused() {
    let memory = Memory::new();
    seed_tickets(&memory).await;
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");
    let ctx = Context::new(memory);
    for (args, message) in [
        ("first: 1, last: 1", "You can pass either `first` or `last`, not both"),
        (r#"first: 1, before: "x""#, "You can pass either `first` and `after` cursor, or `last` and `before` cursor"),
        (r#"last: 1, after: "x""#, "You can pass either `first` and `after` cursor, or `last` and `before` cursor"),
        ("last: 1", "You can pass `last` only with `before` cursor"),
        ("first: 0", "`first` must be a positive integer"),
    ] {
        let query = format!("{{ listTickets({args}) {{ results {{ id }} }} }}");
        let res = schema.execute(Request::new(query).data(ctx.clone())).await;
        assert_eq!(res.errors.first().map(|e| e.message.as_str()), Some(message), "{args}");
    }
}

/// Memory, with every read taking a while, noting the most reads ever under way at once.
#[derive(Clone, Default)]
struct Slow {
    inner: Memory,
    reading: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    most: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl Slow {
    async fn read<T>(&self, read: impl Future<Output = T>) -> T {
        use std::sync::atomic::Ordering::SeqCst;
        let now = self.reading.fetch_add(1, SeqCst) + 1;
        self.most.fetch_max(now, SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let out = read.await;
        self.reading.fetch_sub(1, SeqCst);
        out
    }
}

impl DataLayer for Slow {
    async fn create(&self, resource: &ResourceDef, tenant: Option<&str>, id: ash_core::Value, fields: FieldMap) -> ash_core::Result<FieldMap> {
        self.inner.create(resource, tenant, id, fields).await
    }

    async fn update(&self, resource: &ResourceDef, tenant: Option<&str>, id: ash_core::Value, fields: FieldMap) -> ash_core::Result<FieldMap> {
        self.inner.update(resource, tenant, id, fields).await
    }

    async fn destroy(&self, resource: &ResourceDef, tenant: Option<&str>, id: ash_core::Value) -> ash_core::Result<()> {
        self.inner.destroy(resource, tenant, id).await
    }

    async fn run_query(&self, resource: &ResourceDef, query: &ash_core::CompiledQuery) -> ash_core::Result<Vec<FieldMap>> {
        self.read(self.inner.run_query(resource, query)).await
    }

    async fn count(&self, resource: &ResourceDef, query: &ash_core::CompiledQuery) -> ash_core::Result<usize> {
        self.read(self.inner.count(resource, query)).await
    }
}

/// A page's count is read alongside the page, as Ash reads it, not before it.
#[tokio::test]
async fn a_page_reads_its_count_alongside_its_records() {
    let slow = Slow::default();
    seed_tickets(&slow.inner).await;
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Slow>()
        .expect("Failed to build schema");
    let query = "{ listTickets(sort: [{ field: PRIORITY }], first: 2) { count results { priority } } }";
    let res = schema.execute(Request::new(query).data(Context::new(slow.clone()))).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let page = res.data.into_json().unwrap()["listTickets"].clone();
    assert_eq!(page["count"], 5);
    assert_eq!(page["results"].as_array().unwrap().len(), 2);
    assert_eq!(slow.most.load(std::sync::atomic::Ordering::SeqCst), 2, "the count and the page are read at once");
}
