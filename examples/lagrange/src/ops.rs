//! Workflows that span resources. Each runs in one transaction, and every write goes
//! through a resource action, so policies, validations, state machines and notifiers
//! all apply.

use ash_core::{
    Context, DataLayer, Date, Error, Filter, ResourceExt, Result, TransactionSupport,
    UtcDateTime,
};
use uuid::Uuid;

use crate::types::ReservationStatus;
use crate::{
    Berth, BerthReservation, Container, Contract, Planet, Port, Route, Ship, Stowage, Voyage,
};

fn refused(field: &str, message: impl Into<String>) -> Error {
    Error::validation(field, message, Vec::new())
}

/// Holds `berth_code` at `port_id` for a ship's docking window.
///
/// Reads the berth, checks every line's active reservations for an overlap, claims the
/// berth (raising its lock version), then writes the reservation, all in one
/// transaction. A dispatcher racing for the same berth fails on the claim; the berth
/// can only be held once per window.
pub async fn reserve_berth<D>(
    ctx: &Context<D>,
    ship_id: Uuid,
    port_id: Uuid,
    berth_code: &str,
    starts_at: UtcDateTime,
    ends_at: UtcDateTime,
) -> Result<BerthReservation>
where
    D: DataLayer + TransactionSupport + Clone + 'static,
{
    let berth_code = berth_code.to_string();
    ctx.transaction(move |tx| async move {
        let ship = Ship::get(&tx, ship_id).await?;
        let berth = Berth::get_by_port_code(&tx, port_id, berth_code.clone()).await?;
        if ship.dry_mass_tonnes > berth.max_mass_tonnes {
            return Err(refused(
                "berth",
                format!(
                    "{} t is too heavy for berth {} ({} t)",
                    ship.dry_mass_tonnes, berth.code, berth.max_mass_tonnes
                ),
            ));
        }

        // Reservations belong to lines, but a berth is shared: look across all of them.
        let anyone = tx.clone().without_tenant();
        let overlapping = BerthReservation::query(&anyone)
            .filter(
                BerthReservation::port_id.eq(port_id)
                    & BerthReservation::berth_code.eq(berth_code.clone())
                    & BerthReservation::status.eq(ReservationStatus::Active)
                    & BerthReservation::starts_at.lt(ends_at.clone())
                    & BerthReservation::ends_at.gt(starts_at.clone()),
            )
            .count()
            .await?;
        if overlapping > 0 {
            return Err(refused(
                "berth",
                format!("berth {} is already held for that window", berth.code),
            ));
        }

        berth.claim_on(&tx).await?;
        BerthReservation::reserve(&tx)
            .port_id(port_id)
            .berth_code(berth_code)
            .ship_id(ship_id)
            .starts_at(starts_at)
            .ends_at(ends_at)
            .await
    })
    .await
}

/// Plans a voyage between two ports, with the route computed from the planets'
/// positions. The ship must be docked and have a captain, who will fly it.
pub async fn plan_voyage<D>(
    ctx: &Context<D>,
    ship: &Ship,
    origin_port_id: Uuid,
    destination_port_id: Uuid,
    launch_window: Date,
) -> Result<Voyage>
where
    D: DataLayer + TransactionSupport + Clone + 'static,
{
    if ship.current_state() != "docked" {
        return Err(refused(
            "ship",
            format!("{} is {}, not docked", ship.name, ship.current_state()),
        ));
    }
    let captain_id = ship
        .captain_id
        .ok_or_else(|| refused("ship", format!("{} has no captain", ship.name)))?;
    let origin = Port::get(ctx, origin_port_id).await?;
    let destination = Port::get(ctx, destination_port_id).await?;
    let route: Route = Planet::route(ctx)
        .from_planet(origin.planet_id)
        .to_planet(destination.planet_id)
        .await?;
    Voyage::plan(ctx)
        .ship_id(ship.id)
        .captain_id(captain_id)
        .origin_port_id(origin.id)
        .destination_port_id(destination.id)
        .launch_window(launch_window)
        .distance_au(ash_core::Float::parse(&route.distance_au.to_string())?)
        .transit_hours(route.transit_hours)
        .await
}

/// Where a container goes in the hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub bay: i64,
    pub stack: i64,
    pub tier: i64,
}

/// Stows a container on a planned voyage and marks its contract loaded.
///
/// The container's contract must run between the voyage's ports, and the ship must
/// have a free slot.
pub async fn stow<D>(
    ctx: &Context<D>,
    voyage_id: Uuid,
    container_id: Uuid,
    slot: Slot,
) -> Result<Stowage>
where
    D: DataLayer + TransactionSupport + Clone + 'static,
{
    ctx.transaction(move |tx| async move {
        let voyage = Voyage::query(&tx)
            .filter(Voyage::id.eq(voyage_id))
            .load_aggregate(Voyage::container_count)
            .one()
            .await?;
        if voyage.current_state() != "planned" {
            return Err(refused("voyage", "cargo is only loaded while a voyage is planned"));
        }
        let ship = Ship::get(&tx, voyage.ship_id).await?;
        if voyage.container_count.unwrap_or(0) >= ship.slot_capacity {
            return Err(refused(
                "voyage",
                format!("{} has no free slots ({} aboard)", ship.name, ship.slot_capacity),
            ));
        }
        let container = Container::get(&tx, container_id).await?;
        let contract = Contract::get(&tx, container.contract_id).await?;
        if contract.origin_port_id != voyage.origin_port_id
            || contract.destination_port_id != voyage.destination_port_id
        {
            return Err(refused(
                "container",
                format!("{} is booked on a different route", container.code.as_str()),
            ));
        }

        let stowed = Stowage::stow(&tx)
            .voyage_id(voyage.id)
            .container_id(container.id)
            .bay(slot.bay)
            .stack(slot.stack)
            .tier(slot.tier)
            .await?;
        if contract.current_state() == "booked" {
            contract.load_on(&tx).await?;
        }
        Ok(stowed)
    })
    .await
}

/// Customs clears a voyage once every container of dangerous goods aboard is sealed.
pub async fn clear_customs<D>(ctx: &Context<D>, voyage_id: Uuid) -> Result<Voyage>
where
    D: DataLayer + TransactionSupport + Clone + 'static,
{
    let voyage = Voyage::query(ctx)
        .filter(Voyage::id.eq(voyage_id))
        .load_rel(Voyage::containers)
        .one()
        .await?;
    let unsealed: Vec<String> = voyage
        .containers
        .loaded()?
        .iter()
        .filter(|container| container.hazardous && !container.sealed)
        .map(|container| container.code.as_str().to_string())
        .collect();
    if !unsealed.is_empty() {
        return Err(refused(
            "voyage",
            format!("dangerous goods without a seal: {}", unsealed.join(", ")),
        ));
    }
    voyage.clear_customs_on(ctx).await
}

/// The contracts whose containers ride on a voyage.
async fn contracts_aboard<D: DataLayer>(ctx: &Context<D>, voyage_id: Uuid) -> Result<Vec<Contract>> {
    let stowed = Stowage::query(ctx)
        .filter(Stowage::voyage_id.eq(voyage_id))
        .load_rel(Stowage::container)
        .all()
        .await?;
    let mut contract_ids: Vec<Uuid> = stowed
        .iter()
        .filter_map(|entry| entry.container.loaded().ok().and_then(Option::as_ref))
        .map(|container| container.contract_id)
        .collect();
    contract_ids.sort();
    contract_ids.dedup();
    if contract_ids.is_empty() {
        return Ok(Vec::new());
    }
    Contract::query(ctx)
        .filter(Filter::in_list("id", contract_ids))
        .all()
        .await
}

/// The captain launches: the voyage leaves, the ship is in transit, and its cargo is
/// dispatched. Launching needs a berth held at the destination.
pub async fn launch<D>(ctx: &Context<D>, voyage_id: Uuid, departed_at: UtcDateTime) -> Result<Voyage>
where
    D: DataLayer + TransactionSupport + Clone + 'static,
{
    ctx.transaction(move |tx| async move {
        let voyage = Voyage::get(&tx, voyage_id).await?;
        let launched = voyage.launch_on(&tx).departed_at(departed_at).await?;
        let ship = Ship::get(&tx, launched.ship_id).await?;
        ship.depart_on(&tx).await?;
        for contract in contracts_aboard(&tx, voyage_id).await? {
            if contract.current_state() == "loaded" {
                contract.dispatch_on(&tx).await?;
            }
        }
        Ok(launched)
    })
    .await
}

/// The ship docks at its destination and its cargo is delivered, which emails each
/// shipper.
pub async fn arrive<D>(ctx: &Context<D>, voyage_id: Uuid, arrived_at: UtcDateTime) -> Result<Voyage>
where
    D: DataLayer + TransactionSupport + Clone + 'static,
{
    ctx.transaction(move |tx| async move {
        let voyage = Voyage::get(&tx, voyage_id).await?;
        let arrived = voyage.arrive_on(&tx).arrived_at(arrived_at).await?;
        let ship = Ship::get(&tx, arrived.ship_id).await?;
        ship.arrive_on(&tx).await?;
        for contract in contracts_aboard(&tx, voyage_id).await? {
            if contract.current_state() == "in_transit" {
                contract.deliver_on(&tx).await?;
            }
        }
        Ok(arrived)
    })
    .await
}

/// The dispatcher closes out a voyage once it is unloaded: the berth is released and
/// the contracts close.
pub async fn complete<D>(ctx: &Context<D>, voyage_id: Uuid) -> Result<Voyage>
where
    D: DataLayer + TransactionSupport + Clone + 'static,
{
    ctx.transaction(move |tx| async move {
        let voyage = Voyage::get(&tx, voyage_id).await?;
        let completed = voyage.complete_on(&tx).await?;
        if let Some(reservation_id) = completed.reservation_id {
            BerthReservation::get(&tx, reservation_id)
                .await?
                .release_on(&tx)
                .await?;
        }
        for contract in contracts_aboard(&tx, voyage_id).await? {
            if contract.current_state() == "delivered" {
                contract.close_on(&tx).await?;
            }
        }
        Ok(completed)
    })
    .await
}

/// Retires a ship. `#[archival]` keeps the row, hides it from reads, and cancels its
/// berth reservations.
pub async fn decommission<D>(ctx: &Context<D>, ship_id: Uuid) -> Result<()>
where
    D: DataLayer + TransactionSupport + Clone + 'static,
{
    Ship::get(ctx, ship_id).await?.destroy(ctx).await
}
