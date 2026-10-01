//! Booking cargo: re-imports, quotes, bulk packing, the contract lifecycle, and finding
//! ports by name.

mod support;

use ash_core::{BulkCreateOptions, CiString, Date, Decimal, Error, FieldMap, Value};
use ash_state_machine::InvalidTransition;
use lagrange::seed::HELIOS;
use lagrange::{Container, Contract, Port};
use support::{FleetDb, Harness};

fn book<D: FleetDb>(h: &Harness<D>, reference: &str, freight: i64, deliver_by: &str) -> impl std::future::Future<Output = ash_core::Result<Contract>> {
    Contract::book(h.ctx("Grace"))
        .external_ref(reference)
        .shipper_email(CiString::parse("grace@helios-freight.example").unwrap())
        .origin_port_id(h.sol.port("KSC").id)
        .destination_port_id(h.sol.port("OLY").id)
        .freight_credits(freight)
        .insured_value(Decimal::parse("1000.00").unwrap())
        .deliver_by(Date::parse(deliver_by).unwrap())
        .upsert_on(Contract::external, &["freight_credits", "deliver_by"])
        .call()
}

async fn reimports_update_the_booking<D: FleetDb>(h: Harness<D>) {
    let first = book(&h, "MARS-77", 9_000, "2187-06-01").await.unwrap();
    let again = book(&h, "MARS-77", 9_500, "2187-06-15").await.unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.freight_credits, 9_500);
    assert_eq!(again.deliver_by, Date::parse("2187-06-15").unwrap());
    assert_eq!(Contract::query(h.ctx("Ada")).count().await.unwrap(), 1);

    // The reference is unique per line, so another line may reuse it.
    let sally = h.ctx("Sally");
    Contract::book(sally)
        .external_ref("MARS-77")
        .shipper_email(CiString::parse("sally@red-dust-logistics.example").unwrap())
        .origin_port_id(h.sol.port("KSC").id)
        .destination_port_id(h.sol.port("OLY").id)
        .freight_credits(4_000)
        .insured_value(Decimal::parse("10.00").unwrap())
        .deliver_by(Date::parse("2187-07-01").unwrap())
        .await
        .unwrap();
}
on_every_backend!(reimports_update_the_booking);

async fn quotes_and_bulk_packing<D: FleetDb>(h: Harness<D>) {
    let contract = book(&h, "BULK-1", 20_000, "2187-06-01").await.unwrap();

    let mut rush = FieldMap::new();
    rush.insert("surcharge_pct".into(), Value::Int(25));
    let quoted = Contract::query(h.ctx("Ada"))
        .calc_with_args(Contract::rush_quote, rush)
        .one()
        .await
        .unwrap();
    assert_eq!(quoted.rush_quote, Some(25_000));

    let inputs: Vec<FieldMap> = [(8, None), (12, Some("radioactive")), (20, None)]
        .into_iter()
        .enumerate()
        .map(|(n, (mass, hazmat))| {
            let mut fields = FieldMap::new();
            fields.insert("code".into(), Value::String(format!("BULK1-{n}")));
            fields.insert("contract_id".into(), Value::Uuid(contract.id));
            fields.insert("mass_tonnes".into(), Value::Int(mass));
            if let Some(hazmat) = hazmat {
                fields.insert("hazmat".into(), Value::String(hazmat.into()));
            }
            fields
        })
        .collect();
    let packed = ash_core::bulk_create::<Container, _, _, _>(h.ctx("Grace"), "pack", inputs, BulkCreateOptions::default())
        .await
        .unwrap();
    assert_eq!(packed.count, 3);
    let hazardous: Vec<bool> = Container::query(h.ctx("Ada")).sort_by(Container::mass_tonnes, false).all().await.unwrap().iter().map(|c| c.hazardous).collect();
    assert_eq!(hazardous, [false, true, false]);

    let totals = Contract::query(h.ctx("Ada"))
        .load_aggregate(Contract::container_count)
        .load_aggregate(Contract::booked_mass)
        .one()
        .await
        .unwrap();
    assert_eq!((totals.container_count, totals.booked_mass), (Some(3), Some(40)));

    // A container's limits hold however it is packed.
    let mut too_heavy = FieldMap::new();
    too_heavy.insert("code".into(), Value::String("BULK1-X".into()));
    too_heavy.insert("contract_id".into(), Value::Uuid(contract.id));
    too_heavy.insert("mass_tonnes".into(), Value::Int(41));
    let opts = BulkCreateOptions { stop_on_error: false, ..BulkCreateOptions::default() };
    let result = ash_core::bulk_create::<Container, _, _, _>(h.ctx("Grace"), "pack", [too_heavy], opts).await.unwrap();
    assert_eq!((result.count, result.error_count), (0, 1));
}
on_every_backend!(quotes_and_bulk_packing);

async fn the_lifecycle_has_rules<D: FleetDb>(h: Harness<D>) {
    let booked = book(&h, "LIFE-1", 5_000, "2187-06-01").await.unwrap();
    let cancelled = book(&h, "LIFE-2", 5_000, "2187-05-01").await.unwrap();
    let ada = h.ctx("Ada");

    // Shippers book; only dispatchers move contracts along.
    let err = booked.clone().load_on(h.ctx("Grace")).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden), "{err:?}");
    // A captain can't book.
    let err = Contract::book(h.ctx("Yuri"))
        .external_ref("NOPE")
        .shipper_email(CiString::parse("x@example.com").unwrap())
        .origin_port_id(h.sol.port("KSC").id)
        .destination_port_id(h.sol.port("OLY").id)
        .freight_credits(1)
        .insured_value(Decimal::parse("1").unwrap())
        .deliver_by(Date::parse("2187-01-01").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Forbidden), "{err:?}");

    // The state machine refuses skipped steps.
    let err = booked.clone().deliver_on(ada).await.unwrap_err();
    let Error::Extension(inner) = err else { panic!("expected an invalid transition") };
    let invalid = inner.downcast_ref::<InvalidTransition>().expect("an InvalidTransition");
    assert_eq!(invalid.current_state, "booked");

    cancelled.cancel_on(h.ctx("Grace")).await.unwrap();
    let open: Vec<String> = Contract::open(ada).all().await.unwrap().into_iter().map(|c| c.external_ref).collect();
    assert_eq!(open, ["LIFE-1"]);

    // Origin and destination must differ.
    let err = Contract::book(h.ctx("Grace"))
        .external_ref("LOOP-1")
        .shipper_email(CiString::parse("grace@helios-freight.example").unwrap())
        .origin_port_id(h.sol.port("KSC").id)
        .destination_port_id(h.sol.port("KSC").id)
        .freight_credits(1_000)
        .insured_value(Decimal::parse("1").unwrap())
        .deliver_by(Date::parse("2187-01-01").unwrap())
        .await;
    if h.backend != support::Backend::Memory {
        assert!(err.is_err(), "a round trip to the same port is refused by a check");
    }
}
on_every_backend!(the_lifecycle_has_rules);

async fn contracts_page_by_deadline<D: FleetDb>(h: Harness<D>) {
    for (n, day) in ["2187-01-05", "2187-01-01", "2187-01-04", "2187-01-02", "2187-01-03"].into_iter().enumerate() {
        book(&h, &format!("PAGE-{n}"), 1_000, day).await.unwrap();
    }
    let ada = h.ctx("Ada");
    let mut seen = Vec::new();
    let mut after: Option<String> = None;
    loop {
        let page = Contract::query(ada)
            .sort_by(Contract::deliver_by, false)
            .page_keyset(2, after.as_deref(), None)
            .await
            .unwrap();
        seen.extend(page.results.iter().map(|c| c.deliver_by.as_str().to_string()));
        if !page.has_more {
            break;
        }
        after = page.after.clone();
    }
    assert_eq!(seen, ["2187-01-01", "2187-01-02", "2187-01-03", "2187-01-04", "2187-01-05"]);
}
on_every_backend!(contracts_page_by_deadline);

async fn ports_are_searchable<D: FleetDb>(h: Harness<D>) {
    let anyone = h.app.anonymous();
    let mut stations: Vec<String> = Port::query(&anyone).filter(Port::name.contains("Station")).all().await.unwrap().into_iter().map(|p| p.name).collect();
    stations.sort();
    assert_eq!(stations, ["Gateway Station", "Piazzi Station"]);
    // `name` is case-sensitive; the `code` is a CiString and is not.
    assert!(Port::query(&anyone).filter(Port::name.contains("station")).all().await.unwrap().is_empty());
    let piazzi = Port::query(&anyone).filter(Port::code.starts_with("pz")).calc(Port::label).one().await.unwrap();
    assert_eq!(piazzi.label.as_deref(), Some("PZZ Piazzi Station"));
    assert_eq!(Port::get_by_unique_code(&anyone, CiString::parse("pzz").unwrap()).await.unwrap().id, piazzi.id);

    // Only the port authority changes the map.
    let err = piazzi.clone().rename_on(h.ctx("Ada")).name("Piazzi Hub").await.unwrap_err();
    assert!(matches!(err, Error::Forbidden), "{err:?}");
    let renamed = piazzi.rename_on(&h.app.port_authority()).name("Piazzi Hub").await.unwrap();
    assert_eq!(renamed.name, "Piazzi Hub");
    let _ = HELIOS;
}
on_every_backend!(ports_are_searchable);
