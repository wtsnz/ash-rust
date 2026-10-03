use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

use ash_core::{
    BulkCreateOptions, BulkDestroyOptions, Context, Filter, Resource, SyncFnNotifier, Value,
    resource,
};
use ash_memory::Memory;
use ash_sqlite::Sqlite;

resource! {
    Product {
        table "products";

    attributes {
        id: Uuid [pk];
        sku: String;
        title: String;
        price: i64;
        category: String;
    }

    identities {
        identity unique_sku [sku];
    }

    actions {
        create create {
            primary;
            accept [sku, title, price, category];
        }

        read read {
            primary;
        }

        update update {
            primary;
            accept [title, price, category];
        }

        destroy destroy {
            primary;
        }
    }
    }}

async fn setup_sqlite() -> Context<Sqlite> {
    let sqlite = Sqlite::memory().await.unwrap();
    let ctx = Context::new(sqlite);
    ctx.install(&[&Product::DEF]).await.unwrap();
    ctx
}

fn setup_memory() -> Context<Memory> {
    Context::new(Memory::new())
}

#[tokio::test]
async fn test_bulk_create_memory_and_sqlite() {
    let mem_ctx = setup_memory();
    let sql_ctx = setup_sqlite().await;

    // 1. Memory test
    let mem_items = (1..=50).map(|i| {
        [
            ("sku", Value::from(format!("SKU-MEM-{i}"))),
            ("title", Value::from(format!("Product {i}"))),
            ("price", Value::from(i * 10)),
            ("category", Value::from("Electronics")),
        ]
    });

    let res = Product::bulk_create(&mem_ctx, mem_items).await.unwrap();
    assert_eq!(res.count, 50);
    assert_eq!(res.records.len(), 50);
    assert!(res.is_success());

    let count = Product::query(&mem_ctx).load().await.unwrap().len();
    assert_eq!(count, 50);

    // 2. Sqlite test
    let sql_items = (1..=50).map(|i| {
        [
            ("sku", Value::from(format!("SKU-SQL-{i}"))),
            ("title", Value::from(format!("Product {i}"))),
            ("price", Value::from(i * 10)),
            ("category", Value::from("Electronics")),
        ]
    });

    let res = Product::bulk_create(&sql_ctx, sql_items).await.unwrap();
    assert_eq!(res.count, 50);
    assert_eq!(res.records.len(), 50);
    assert!(res.is_success());

    let count = Product::query(&sql_ctx).load().await.unwrap().len();
    assert_eq!(count, 50);
}

#[tokio::test]
async fn test_bulk_create_chunked_batches() {
    let ctx = setup_memory();

    let items = (1..=25).map(|i| {
        [
            ("sku", Value::from(format!("SKU-CHUNK-{i}"))),
            ("title", Value::from(format!("Chunk Item {i}"))),
            ("price", Value::from(100)),
            ("category", Value::from("Books")),
        ]
    });

    let opts = BulkCreateOptions::new().batch_size(10);
    let res = Product::bulk_create_with_opts(&ctx, "create", items, opts).await.unwrap();
    assert_eq!(res.count, 25);
    assert_eq!(res.records.len(), 25);
}

#[tokio::test]
async fn test_bulk_create_with_upsert() {
    let ctx = setup_sqlite().await;

    // Insert 3 products initially
    Product::create(&ctx)
        .sku("SKU-1")
        .title("Old Title 1")
        .price(10)
        .category("Tools")
        .await
        .unwrap();

    let items = vec![
        [
            ("sku", Value::from("SKU-1")),
            ("title", Value::from("Updated Title 1")),
            ("price", Value::from(20)),
            ("category", Value::from("Tools")),
        ],
        [
            ("sku", Value::from("SKU-2")),
            ("title", Value::from("New Product 2")),
            ("price", Value::from(30)),
            ("category", Value::from("Tools")),
        ],
    ];

    let opts = BulkCreateOptions::new().upsert("unique_sku", &["title", "price"]);
    let res = Product::bulk_create_with_opts(&ctx, "create", items, opts).await.unwrap();
    assert_eq!(res.count, 2);

    let p1 = Product::query(&ctx)
        .filter(Filter::eq("sku", "SKU-1"))
        .first()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(p1.title, "Updated Title 1");
    assert_eq!(p1.price, 20);

    let p2 = Product::query(&ctx)
        .filter(Filter::eq("sku", "SKU-2"))
        .first()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(p2.title, "New Product 2");
    assert_eq!(p2.price, 30);
}

#[tokio::test]
async fn test_bulk_destroy_memory_and_sqlite() {
    let mem_ctx = setup_memory();
    let sql_ctx = setup_sqlite().await;

    // Test Memory
    let p1 = Product::create(&mem_ctx).sku("M1").title("T1").price(10).category("C").await.unwrap();
    let p2 = Product::create(&mem_ctx).sku("M2").title("T2").price(10).category("C").await.unwrap();
    let p3 = Product::create(&mem_ctx).sku("M3").title("T3").price(10).category("C").await.unwrap();

    let res = Product::bulk_destroy(&mem_ctx, &[p1.id, p2.id]).await.unwrap();
    assert_eq!(res.count, 2);

    let remaining = Product::query(&mem_ctx).load().await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, p3.id);

    // Test Sqlite
    let s1 = Product::create(&sql_ctx).sku("S1").title("T1").price(10).category("C").await.unwrap();
    let s2 = Product::create(&sql_ctx).sku("S2").title("T2").price(10).category("C").await.unwrap();
    let s3 = Product::create(&sql_ctx).sku("S3").title("T3").price(10).category("C").await.unwrap();

    let res = Product::bulk_destroy(&sql_ctx, &[s1.id, s3.id]).await.unwrap();
    assert_eq!(res.count, 2);

    let remaining = Product::query(&sql_ctx).load().await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, s2.id);
}

#[tokio::test]
async fn test_query_bulk_destroy() {
    let ctx = setup_sqlite().await;

    for i in 1..=10 {
        let cat = if i <= 6 { "DeleteMe" } else { "KeepMe" };
        Product::create(&ctx)
            .sku(format!("SKU-DEL-{i}"))
            .title(format!("Title {i}"))
            .price(10)
            .category(cat)
            .await
            .unwrap();
    }

    let res = Product::query(&ctx)
        .filter(Filter::eq("category", "DeleteMe"))
        .bulk_destroy("destroy", BulkDestroyOptions::default())
        .await
        .unwrap();

    assert_eq!(res.count, 6);

    let remaining = Product::query(&ctx).load().await.unwrap();
    assert_eq!(remaining.len(), 4);
    for r in remaining {
        assert_eq!(r.category, "KeepMe");
    }
}

#[tokio::test]
async fn test_query_chunked_iteration() {
    let ctx = setup_memory();

    for i in 1..=45 {
        Product::create(&ctx)
            .sku(format!("SKU-ITER-{i:02}"))
            .title(format!("Title {i}"))
            .price(i)
            .category("General")
            .await
            .unwrap();
    }

    let chunk_counts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let counts_clone = chunk_counts.clone();

    let total = Product::query(&ctx)
        .sort(Product::price)
        .chunked(10, |chunk| {
            let counts = counts_clone.clone();
            async move {
                counts.lock().unwrap().push(chunk.len());
                Ok(())
            }
        })
        .await
        .unwrap();

    assert_eq!(total, 45);
    let counts = chunk_counts.lock().unwrap().clone();
    assert_eq!(counts, vec![10, 10, 10, 10, 5]);
}

#[tokio::test]
async fn test_query_chunked_keyset_iteration() {
    let ctx = setup_sqlite().await;

    for i in 1..=32 {
        Product::create(&ctx)
            .sku(format!("SKU-KITER-{i:02}"))
            .title(format!("Title {i}"))
            .price(i * 5)
            .category("Keyed")
            .await
            .unwrap();
    }

    let processed_skus = Arc::new(std::sync::Mutex::new(Vec::new()));
    let skus_clone = processed_skus.clone();

    let total = Product::query(&ctx)
        .sort(Product::sku)
        .chunked_keyset(10, |chunk| {
            let skus = skus_clone.clone();
            async move {
                let mut guard = skus.lock().unwrap();
                for item in chunk {
                    guard.push(item.sku);
                }
                Ok(())
            }
        })
        .await
        .unwrap();

    assert_eq!(total, 32);
    let recorded = processed_skus.lock().unwrap().clone();
    assert_eq!(recorded.len(), 32);
    assert_eq!(recorded.first().unwrap(), "SKU-KITER-01");
    assert_eq!(recorded.last().unwrap(), "SKU-KITER-32");
}

#[tokio::test]
async fn test_bulk_operations_with_notifiers() {
    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = counter.clone();

    let notifier = Arc::new(SyncFnNotifier::new("test_notifier", move |_notification| {
        counter_clone.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }));

    let ctx = setup_memory().with_notifier(notifier);

    let items = (1..=10).map(|i| {
        [
            ("sku", Value::from(format!("SKU-NOTIF-{i}"))),
            ("title", Value::from(format!("Item {i}"))),
            ("price", Value::from(50)),
            ("category", Value::from("Notif")),
        ]
    });

    let created = Product::bulk_create(&ctx, items).await.unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 10);

    let ids: Vec<Uuid> = created.records.iter().map(|r| r.id).collect();
    Product::bulk_destroy(&ctx, &ids).await.unwrap();
    assert_eq!(counter.load(Ordering::SeqCst), 20);
}

#[tokio::test]
async fn test_multi_bulk_operations() {
    let ctx = setup_sqlite().await;

    let items = vec![
        [
            ("sku", Value::from("SKU-MULTI-1")),
            ("title", Value::from("Multi Item 1")),
            ("price", Value::from(100)),
            ("category", Value::from("Multi")),
        ],
        [
            ("sku", Value::from("SKU-MULTI-2")),
            ("title", Value::from("Multi Item 2")),
            ("price", Value::from(200)),
            ("category", Value::from("Multi")),
        ],
    ];

    let result = ctx
        .multi()
        .bulk_create::<Product, _, _>(
            "create_products",
            "create",
            items,
            BulkCreateOptions::default(),
        )
        .commit()
        .await
        .unwrap();

    let bulk_res: &ash_core::BulkResult<Product> = result.get("create_products").unwrap();
    assert_eq!(bulk_res.count, 2);

    let p1_id = bulk_res.records[0].id;
    let p2_id = bulk_res.records[1].id;

    // Test multi bulk destroy
    let del_result = ctx
        .multi()
        .bulk_destroy::<Product>(
            "destroy_products",
            "destroy",
            vec![p1_id, p2_id],
            BulkDestroyOptions::default(),
        )
        .commit()
        .await
        .unwrap();

    let del_res: &ash_core::BulkResult<Product> = del_result.get("destroy_products").unwrap();
    assert_eq!(del_res.count, 2);

    let count = Product::query(&ctx).load().await.unwrap().len();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn test_bulk_create_stop_on_error_false() {
    let ctx = setup_memory();

    let items = vec![
        [
            ("sku", Value::from("SKU-VALID-1")),
            ("title", Value::from("Item 1")),
            ("price", Value::from(10)),
            ("category", Value::from("General")),
        ],
        // Missing required title
        [
            ("sku", Value::from("SKU-INVALID-2")),
            ("title", Value::Null),
            ("price", Value::from(20)),
            ("category", Value::from("General")),
        ],
        [
            ("sku", Value::from("SKU-VALID-3")),
            ("title", Value::from("Item 3")),
            ("price", Value::from(30)),
            ("category", Value::from("General")),
        ],
    ];

    let opts = BulkCreateOptions::new().stop_on_error(false);
    let res = Product::bulk_create_with_opts(&ctx, "create", items, opts).await.unwrap();

    assert_eq!(res.count, 2);
    assert_eq!(res.records.len(), 2);
    assert_eq!(res.error_count, 1);
    assert!(!res.is_success());
    assert_eq!(res.errors.len(), 1);
}

/// Each record gets its own input, through the update action's changeset; the batch is
/// written together and each row is notified as an update. A row that fails validation
/// fails alone when the bulk update carries on past errors.
async fn bulk_updates_each_record<D: ash_core::TransactionSupport + 'static>(ctx: Context<D>) {
    let updates = Arc::new(AtomicUsize::new(0));
    let heard = updates.clone();
    let ctx = ctx.with_notifier(Arc::new(SyncFnNotifier::new("updates", move |notification| {
        if notification.action_kind == ash_core::ActionKind::Update {
            heard.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    })));
    let created = Product::bulk_create(
        &ctx,
        (1..=4).map(|i| {
            [
                ("sku", Value::from(format!("SKU-UP-{i}"))),
                ("title", Value::from(format!("Item {i}"))),
                ("price", Value::from(i * 10)),
                ("category", Value::from("Before")),
            ]
        }),
    )
    .await
    .unwrap();

    // Each its own price; the third also moves category, and a stale copy's title isn't
    // written back.
    let mut stale = created.records.clone();
    let renamed = stale[1].clone().update_on(&ctx).title("Renamed".to_string()).await.unwrap();
    assert_eq!(renamed.title, "Renamed");
    stale[2].category = "Stale".into();
    let inputs = stale.into_iter().enumerate().map(|(i, product)| {
        let mut input = ash_core::FieldMap::new();
        input.insert("price".into(), Value::from(1_000 + i as i64));
        if i == 2 {
            input.insert("category".into(), Value::from("After"));
        }
        (product, input)
    });
    let result = Product::bulk_update(&ctx, "update", inputs).await.unwrap();
    assert_eq!(result.count, 4);
    assert_eq!(updates.load(Ordering::SeqCst), 5, "the rename, then one per row");

    let mut stored = Product::query(&ctx).load().await.unwrap();
    stored.sort_by_key(|p| p.price);
    let summary: Vec<(i64, &str, &str)> = stored
        .iter()
        .map(|p| (p.price, p.title.as_str(), p.category.as_str()))
        .collect();
    assert_eq!(
        summary,
        [
            (1_000, "Item 1", "Before"),
            (1_001, "Renamed", "Before"),
            (1_002, "Item 3", "After"),
            (1_003, "Item 4", "Before"),
        ]
    );

    // One row's input isn't accepted by the action: it fails alone.
    let inputs = stored.into_iter().enumerate().map(|(i, product)| {
        let mut input = ash_core::FieldMap::new();
        input.insert("price".into(), Value::from(2_000 + i as i64));
        if i == 0 {
            input.insert("sku".into(), Value::from("NOT-ACCEPTED"));
        }
        (product, input)
    });
    let opts = ash_core::BulkUpdateOptions::new().stop_on_error(false);
    let result = Product::bulk_update_with_opts(&ctx, "update", inputs, opts).await.unwrap();
    assert_eq!((result.count, result.error_count), (3, 1), "{:?}", result.errors);
    let prices: Vec<i64> = {
        let mut prices: Vec<i64> = Product::query(&ctx).load().await.unwrap().iter().map(|p| p.price).collect();
        prices.sort_unstable();
        prices
    };
    assert_eq!(prices, [1_000, 2_001, 2_002, 2_003]);
}

#[tokio::test]
async fn test_bulk_update_memory_and_sqlite() {
    bulk_updates_each_record(setup_memory()).await;
    bulk_updates_each_record(setup_sqlite().await).await;
}
