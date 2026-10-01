//! A week in the Sol system, start to finish: two lines book cargo, pack and seal it,
//! fight over a berth, fly, stream telemetry, deliver, and retire a ship.

use std::fmt::Write as _;

use ash_core::{
    BulkCreateOptions, CiString, Context, DataLayer, Date, Decimal, Float, Inet, Result,
    TransactionSupport, UtcDateTime, Value, Vector,
};

use crate::ops::{self, Slot};
use crate::seed::{self, HELIOS, RED_DUST, Sol};
use crate::types::HazmatClass;
use crate::{Container, Contract, Lagrange, OpsEvent, TelemetryPing, Voyage};

/// What the week produced.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub contracts_closed: usize,
    pub containers_delivered: i64,
    pub pings_recorded: usize,
    pub audit_events: usize,
    pub berth_refusals: usize,
    pub ships_retired: usize,
}

fn at(raw: &str) -> Result<UtcDateTime> {
    UtcDateTime::parse(raw)
}

fn ci(raw: &str) -> Result<CiString> {
    CiString::parse(raw)
}

struct Booking<'a> {
    reference: &'a str,
    email: &'a str,
    origin: &'a str,
    destination: &'a str,
    freight: i64,
    deliver_by: &'a str,
}

async fn book<D>(ctx: &Context<D>, sol: &Sol, booking: Booking<'_>) -> Result<Contract>
where
    D: DataLayer + TransactionSupport + Clone + Send + Sync + 'static,
{
    Contract::book(ctx)
        .external_ref(booking.reference)
        .shipper_email(ci(booking.email)?)
        .origin_port_id(sol.port(booking.origin).id)
        .destination_port_id(sol.port(booking.destination).id)
        .freight_credits(booking.freight)
        .insured_value(Decimal::parse("50000.00")?)
        .deliver_by(Date::parse(booking.deliver_by)?)
        .upsert_on(Contract::external, &["freight_credits", "deliver_by"])
        .call()
        .await
}

/// Packs containers of the given masses into a contract in one bulk call.
async fn pack<D>(ctx: &Context<D>, contract: &Contract, prefix: &str, masses: &[i64]) -> Result<Vec<Container>>
where
    D: DataLayer + TransactionSupport + Clone + Send + Sync + 'static,
{
    let inputs: Vec<ash_core::FieldMap> = masses
        .iter()
        .enumerate()
        .map(|(n, mass)| {
            let mut fields = ash_core::FieldMap::new();
            fields.insert("code".into(), Value::String(format!("{prefix}{n:04}")));
            fields.insert("contract_id".into(), Value::Uuid(contract.id));
            fields.insert("mass_tonnes".into(), Value::Int(*mass));
            fields
        })
        .collect();
    let options = BulkCreateOptions {
        return_records: true,
        ..BulkCreateOptions::default()
    };
    Ok(ash_core::bulk_create::<Container, _, _, _>(ctx, "pack", inputs, options)
        .await?
        .records)
}

/// Flies a voyage that is loaded and holds a berth: customs, launch, a stream of
/// telemetry, arrival, and close-out.
async fn fly<D>(
    app: &Lagrange<D>,
    sol: &Sol,
    voyage: &Voyage,
    captain: &str,
    ship: &str,
    log: &mut String,
) -> Result<usize>
where
    D: DataLayer + TransactionSupport + Clone + Send + Sync + 'static,
{
    let crew = seed::contexts(app, sol);
    let customs = crew["Chen"].clone().with_tenant(voyage.line.clone());
    ops::clear_customs(&customs, voyage.id).await?;
    ops::launch(&crew[captain], voyage.id, at("2187-02-01T06:00:00Z")?).await?;
    writeln!(log, "  {ship} launched ({:.2} AU, {} h)", voyage.distance_au.as_str().parse::<f64>().unwrap_or(0.0), voyage.transit_hours).ok();

    let transponder = app.transponder(&sol.api_keys[ship]).await?;
    let pings = 12;
    for hour in 0..pings {
        TelemetryPing::report(&transponder)
            .position(Vector::new([1.0 - hour as f32 * 0.05, hour as f32 * 0.2, 0.01])?)
            .speed_kms(Float::parse("38.5")?)
            .fuel_pct(95 - hour)
            .recorded_at(at(&format!("2187-02-01T{:02}:00:00Z", 7 + hour))?)
            .source_ip(Inet::parse("10.0.2.1")?)
            .await?;
    }

    ops::arrive(&crew[captain], voyage.id, at("2187-02-12T03:00:00Z")?).await?;
    let dispatcher = if voyage.line == HELIOS { "Ada" } else { "Mae" };
    ops::complete(&crew[dispatcher], voyage.id).await?;
    writeln!(log, "  {ship} arrived and unloaded; {pings} pings on file").ok();
    Ok(pings as usize)
}

/// Runs the week on a fresh fleet database and returns what happened, with a log of it.
pub async fn week_in_sol<D>(app: &Lagrange<D>) -> Result<(Summary, String)>
where
    D: DataLayer + TransactionSupport + Clone + Send + Sync + 'static,
{
    let mut log = String::new();
    let mut summary = Summary::default();
    let sol = seed::sol(app).await?;
    let crew = seed::contexts(app, &sol);
    writeln!(log, "Charted {} bodies and {} ports; {} ships in service.", sol.planets.len(), sol.ports.len(), sol.ships.len()).ok();

    // Monday: bookings. Grace's system sends one booking twice with a better rate.
    book(&crew["Grace"], &sol, Booking { reference: "HF-CERES-1", email: "grace@helios-freight.example", origin: "LEO", destination: "PZZ", freight: 18_000, deliver_by: "2187-03-01" }).await?;
    let ceres = book(&crew["Grace"], &sol, Booking { reference: "HF-CERES-1", email: "grace@helios-freight.example", origin: "LEO", destination: "PZZ", freight: 16_500, deliver_by: "2187-03-01" }).await?;
    let mars = book(&crew["Sally"], &sol, Booking { reference: "RD-OLY-1", email: "sally@red-dust-logistics.example", origin: "KSC", destination: "OLY", freight: 9_000, deliver_by: "2187-03-10" }).await?;
    writeln!(log, "Monday: {} and {} booked; {} re-imported at {} credits.", ceres.external_ref, mars.external_ref, ceres.external_ref, ceres.freight_credits).ok();

    // Tuesday: packing, and a drum of reactor fuel that customs must seal.
    let mut helios_cargo = pack(&crew["Grace"], &ceres, "HFCU", &[22, 18, 26]).await?;
    let fuel = Container::pack(&crew["Grace"])
        .code(ci("HFRX0001")?)
        .contract_id(ceres.id)
        .mass_tonnes(12)
        .hazmat(HazmatClass::Radioactive)
        .await?;
    helios_cargo.push(fuel.clone());
    let red_dust_cargo = pack(&crew["Sally"], &mars, "RDCU", &[30, 30]).await?;
    writeln!(log, "Tuesday: packed {} containers for Helios and {} for Red Dust.", helios_cargo.len(), red_dust_cargo.len()).ok();

    // Wednesday: voyages, stowage, and berths at the destination.
    let long_haul = ops::plan_voyage(&crew["Ada"], sol.ship("Long Haul"), sol.port("LEO").id, sol.port("PZZ").id, Date::parse("2187-02-01")?).await?;
    let dust_devil = ops::plan_voyage(&crew["Mae"], sol.ship("Dust Devil"), sol.port("KSC").id, sol.port("OLY").id, Date::parse("2187-02-01")?).await?;
    for (n, container) in helios_cargo.iter().enumerate() {
        ops::stow(&crew["Ada"], long_haul.id, container.id, Slot { bay: 1, stack: 1, tier: n as i64 + 1 }).await?;
    }
    for (n, container) in red_dust_cargo.iter().enumerate() {
        ops::stow(&crew["Mae"], dust_devil.id, container.id, Slot { bay: 1, stack: 1, tier: n as i64 + 1 }).await?;
    }

    let window = (at("2187-02-11T12:00:00Z")?, at("2187-02-12T12:00:00Z")?);
    let held = ops::reserve_berth(&crew["Mae"], sol.ship("Dust Devil").id, sol.port("OLY").id, "A1", window.0.clone(), window.1.clone()).await?;
    // Helios wants the same berth for the same window and is turned away.
    if ops::reserve_berth(&crew["Ada"], sol.ship("Tin Kettle").id, sol.port("OLY").id, "A1", window.0.clone(), window.1.clone()).await.is_err() {
        summary.berth_refusals += 1;
    }
    let ceres_berth = ops::reserve_berth(&crew["Ada"], sol.ship("Long Haul").id, sol.port("PZZ").id, "A1", window.0, window.1).await?;
    let long_haul = Voyage::get(&crew["Ada"], long_haul.id).await?.hold_berth_on(&crew["Ada"]).reservation_id(ceres_berth.id).await?;
    let dust_devil = Voyage::get(&crew["Mae"], dust_devil.id).await?.hold_berth_on(&crew["Mae"]).reservation_id(held.id).await?;
    writeln!(log, "Wednesday: voyages planned and loaded; {} berth request refused as taken.", summary.berth_refusals).ok();

    // Thursday: customs refuses the unsealed fuel, seals it, and clears both voyages.
    let customs = crew["Chen"].clone().with_tenant(HELIOS);
    let refused = ops::clear_customs(&customs, long_haul.id).await.is_err();
    let seal = ash_core::Binary::from_bytes(*b"SOL-CUSTOMS 2187-01-29 #4471");
    Container::get(&customs, fuel.id).await?.seal_on(&customs).seal(seal).await?;
    writeln!(log, "Thursday: customs {} the fuel drum, then sealed it.", if refused { "refused" } else { "missed" }).ok();

    // Friday to the 12th: both ships fly.
    summary.pings_recorded += fly(app, &sol, &long_haul, "Yuri", "Long Haul", &mut log).await?;
    summary.pings_recorded += fly(app, &sol, &dust_devil, "Neil", "Dust Devil", &mut log).await?;

    // Saturday: the tanker is retired.
    ops::decommission(&crew["Ada"], sol.ship("Behemoth").id).await?;
    summary.ships_retired = 1;
    writeln!(log, "Saturday: Behemoth decommissioned.").ok();

    for line in [HELIOS, RED_DUST] {
        let dispatcher = if line == HELIOS { "Ada" } else { "Mae" };
        let closed = Contract::query(&crew[dispatcher])
            .filter(Contract::status.eq("closed"))
            .load_aggregate(Contract::container_count)
            .all()
            .await?;
        summary.contracts_closed += closed.len();
        summary.containers_delivered += closed.iter().filter_map(|c| c.container_count).sum::<i64>();
    }
    let audit = Context::new(app.registry());
    summary.audit_events = OpsEvent::query(&audit).count().await?;
    writeln!(log, "Week done: {summary:?}").ok();
    Ok((summary, log))
}
