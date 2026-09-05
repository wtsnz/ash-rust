use ash_core::{resource, Context, FieldMap, Resource, Result, SchemaSupport, Value};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

fn calculate_discount_code(fields: &FieldMap) -> Result<Value> {
    let price = fields.get("price").and_then(|v| v.as_int()).unwrap_or(0);
    if price > 50 {
        Ok(Value::String("VIP-DISCOUNT".to_string()))
    } else {
        Ok(Value::String("STANDARD".to_string()))
    }
}

pub mod item {
    use super::*;

    resource! {
        resource Item;
        table "items";

        attributes {
            id: Uuid [pk],
            name: String,
            code: String,
            price: i64,
            quantity: i64,
            views: i64 = 0,
            nickname: Option<String>,
        }

        calculations {
            total_price: i64 = price * quantity;
            bonus_views: i64 = views + 100;
            name_len: i64 = length(name);
            upper_code: String = upper(code);
            display_name: String = coalesce(nickname, name, "Item");
            badge: String = if_else(views >= 500, "Popular", "Regular");
            discount_type: String = custom(calculate_discount_code);
            discounted(discount: i64): i64 = price - arg(discount);
        }

        actions {
            create create {
                primary;
                accept [name, code, price, quantity, views, nickname];
            }

            read read {
                primary;
            }
        }
    }
}

pub use item::Item;

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_calculations_in_memory() {
    let data = Memory::new();
    let ctx = Context::new(data);

    let _item1 = Item::create(&ctx)
        .name("Laptop")
        .code("lp-100")
        .price(1000)
        .quantity(2)
        .views(800)
        .call()
        .await
        .unwrap();

    let _item2 = Item::create(&ctx)
        .name("Mouse")
        .code("ms-20")
        .price(25)
        .quantity(4)
        .views(50)
        .nickname("Clicky")
        .call()
        .await
        .unwrap();

    let _item3 = Item::create(&ctx)
        .name("Keyboard")
        .code("kb-50")
        .price(45)
        .quantity(1)
        .views(200)
        .call()
        .await
        .unwrap();

    // Invariant 1: Loading calculations populates calculated values
    let results = Item::query(&ctx)
        .calc(Item::total_price)
        .calc(Item::bonus_views)
        .calc(Item::name_len)
        .calc(Item::upper_code)
        .calc(Item::display_name)
        .calc(Item::badge)
        .calc(Item::discount_type)
        .all()
        .await
        .unwrap();

    assert_eq!(results.len(), 3);

    // Verify item 1 (Laptop)
    let laptop = results.iter().find(|i| i.name == "Laptop").unwrap();
    assert_eq!(laptop.total_price, Some(2000)); // 1000 * 2
    assert_eq!(laptop.bonus_views, Some(900));  // 800 + 100
    assert_eq!(laptop.name_len, Some(6));      // "Laptop".len()
    assert_eq!(laptop.upper_code, Some("LP-100".into()));
    assert_eq!(laptop.display_name, Some("Laptop".into())); // null nickname -> name
    assert_eq!(laptop.badge, Some("Popular".into())); // views >= 500
    assert_eq!(laptop.discount_type, Some("VIP-DISCOUNT".into())); // price > 50

    // Verify item 2 (Mouse)
    let mouse = results.iter().find(|i| i.name == "Mouse").unwrap();
    assert_eq!(mouse.total_price, Some(100)); // 25 * 4
    assert_eq!(mouse.display_name, Some("Clicky".into())); // non-null nickname
    assert_eq!(mouse.badge, Some("Regular".into())); // views < 500
    assert_eq!(mouse.discount_type, Some("STANDARD".into())); // price <= 50

    // Invariant 2: Filtering on calculated fields in memory
    let high_total = Item::query(&ctx)
        .calc(Item::total_price)
        .filter(Item::total_price.gt(500))
        .all()
        .await
        .unwrap();
    assert_eq!(high_total.len(), 1);
    assert_eq!(high_total[0].name, "Laptop");

    // Invariant 3: Sorting by calculated fields in memory
    let sorted_by_total = Item::query(&ctx)
        .calc(Item::total_price)
        .sort_desc(Item::total_price)
        .all()
        .await
        .unwrap();
    assert_eq!(sorted_by_total[0].name, "Laptop");   // 2000
    assert_eq!(sorted_by_total[1].name, "Mouse");    // 100
    assert_eq!(sorted_by_total[2].name, "Keyboard"); // 45

    // Invariant 4: Calculation with runtime arguments
    let mut args = FieldMap::new();
    args.insert("discount".to_string(), Value::Int(100));
    let discounted_items = Item::query(&ctx)
        .calc_with_args(Item::discounted, args)
        .all()
        .await
        .unwrap();
    let laptop = discounted_items.iter().find(|i| i.name == "Laptop").unwrap();
    assert_eq!(laptop.discounted, Some(900)); // 1000 - 100
    let mouse = discounted_items.iter().find(|i| i.name == "Mouse").unwrap();
    assert_eq!(mouse.discounted, Some(-75)); // 25 - 100
}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_calculations_in_sqlite() {
    let data = Sqlite::memory().await.unwrap();
    data.install_resources(&[&Item::DEF]).await.unwrap();
    let ctx = Context::new(data);

    Item::create(&ctx)
        .name("Laptop")
        .code("lp-100")
        .price(1000)
        .quantity(2)
        .views(800)
        .call()
        .await
        .unwrap();

    Item::create(&ctx)
        .name("Mouse")
        .code("ms-20")
        .price(25)
        .quantity(4)
        .views(50)
        .nickname("Clicky")
        .call()
        .await
        .unwrap();

    Item::create(&ctx)
        .name("Keyboard")
        .code("kb-50")
        .price(45)
        .quantity(1)
        .views(200)
        .call()
        .await
        .unwrap();

    // Invariant 1: SQL generation for SELECT calculations (arithmetic, functions, conditional)
    let results = Item::query(&ctx)
        .calc(Item::total_price)
        .calc(Item::bonus_views)
        .calc(Item::name_len)
        .calc(Item::upper_code)
        .calc(Item::display_name)
        .calc(Item::badge)
        .all()
        .await
        .unwrap();

    assert_eq!(results.len(), 3);
    let laptop = results.iter().find(|i| i.name == "Laptop").unwrap();
    assert_eq!(laptop.total_price, Some(2000));
    assert_eq!(laptop.bonus_views, Some(900));
    assert_eq!(laptop.name_len, Some(6));
    assert_eq!(laptop.upper_code, Some("LP-100".into()));
    assert_eq!(laptop.display_name, Some("Laptop".into()));
    assert_eq!(laptop.badge, Some("Popular".into()));

    // Invariant 2: SQL generation for WHERE with calculations
    let high_total = Item::query(&ctx)
        .calc(Item::total_price)
        .filter(Item::total_price.gt(500))
        .all()
        .await
        .unwrap();
    assert_eq!(high_total.len(), 1);
    assert_eq!(high_total[0].name, "Laptop");

    // Invariant 3: SQL generation for ORDER BY with calculations
    let sorted_by_total = Item::query(&ctx)
        .calc(Item::total_price)
        .sort_desc(Item::total_price)
        .all()
        .await
        .unwrap();
    assert_eq!(sorted_by_total[0].name, "Laptop");
    assert_eq!(sorted_by_total[1].name, "Mouse");
    assert_eq!(sorted_by_total[2].name, "Keyboard");

    // Invariant 4: Calculation with runtime arguments in SQL
    let mut args = FieldMap::new();
    args.insert("discount".to_string(), Value::Int(100));
    let discounted_items = Item::query(&ctx)
        .calc_with_args(Item::discounted, args)
        .all()
        .await
        .unwrap();
    let laptop = discounted_items.iter().find(|i| i.name == "Laptop").unwrap();
    assert_eq!(laptop.discounted, Some(900)); // 1000 - 100
    let mouse = discounted_items.iter().find(|i| i.name == "Mouse").unwrap();
    assert_eq!(mouse.discounted, Some(-75)); // 25 - 100
}
