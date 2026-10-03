//! The data both apps load: generated from a seed, so every run, and both apps, see the
//! same desk.
//!
//! The fixture is JSON: one array of records per resource (`orgs`, `agents`, `tags`,
//! `tickets`, `comments`, `ticket_tags`), each record its attributes by name, ids and
//! timestamps as strings. The Elixir app reads the same file.

use ash_core::{AttrType, Context, DataLayer, FieldMap, Resource, ResourceDef, Result, Value};
use serde_json::{Map, Value as Json, json};
use uuid::Uuid;

use crate::{Agent, Comment, Org, Tag, Ticket, TicketTag};

/// Orgs, and how each one's desk is staffed and busy: 10 orgs, 10 agents and 20 tags
/// each, 1,000 tickets each with 5 comments on average.
pub const ORGS: &[&str] = &["acme", "globex", "initech", "umbrella", "hooli", "stark", "wayne", "wonka", "tyrell", "soylent"];
const AGENTS_PER_ORG: usize = 10;
const TAGS_PER_ORG: usize = 20;
const TICKETS_PER_ORG: usize = 1_000;
const COMMENTS: usize = 50_000;

const TAG_NAMES: &[&str] = &[
    "billing", "bug", "feature", "login", "outage", "refund", "api", "mobile", "export", "invoice", "sso", "onboarding",
    "performance", "security", "integrations", "email", "reports", "search", "permissions", "notifications",
];
const WORDS: &[&str] = &[
    "cannot", "login", "invoice", "missing", "export", "fails", "slow", "dashboard", "password", "reset", "charge", "twice",
    "error", "upload", "report", "broken", "sync", "calendar", "access", "denied", "refund", "request", "webhook", "timeout",
];
const STATUSES: &[(&str, u64)] = &[("new", 20), ("open", 30), ("pending", 15), ("resolved", 20), ("closed", 15)];

/// SplitMix64: small, fast, and the same in every language, so the fixture can be
/// regenerated anywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn uuid(&mut self) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&self.next().to_be_bytes());
        bytes[8..].copy_from_slice(&self.next().to_be_bytes());
        uuid::Builder::from_random_bytes(bytes).into_uuid()
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }

    fn weighted(&mut self, items: &[(&'static str, u64)]) -> &'static str {
        let total: u64 = items.iter().map(|(_, w)| w).sum();
        let mut roll = self.below(total);
        for (item, weight) in items {
            if roll < *weight {
                return item;
            }
            roll -= weight;
        }
        items[0].0
    }

    fn words(&mut self, min: u64, max: u64) -> String {
        let n = min + self.below(max - min + 1);
        (0..n).map(|_| *self.pick(WORDS)).collect::<Vec<_>>().join(" ")
    }
}

/// `seconds` after the fixture's epoch, as Ash writes a `utc_datetime_usec`.
fn at(seconds: u64) -> String {
    const EPOCH: i64 = 1_767_225_600; // 2026-01-01T00:00:00Z
    let t = EPOCH + seconds as i64;
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000000Z",
        secs / 3_600,
        secs % 3_600 / 60,
        secs % 60
    )
}

/// The desk, from `seed`.
pub fn generate(seed: u64) -> Json {
    let mut rng = Rng(seed);
    let (mut orgs, mut agents, mut tags, mut tickets, mut ticket_tags) = (vec![], vec![], vec![], vec![], vec![]);
    // Each ticket's org, id, staff and when it was opened, for its comments.
    let mut opened: Vec<(&str, Uuid, Vec<Uuid>, u64)> = Vec::new();
    for (o, slug) in ORGS.iter().enumerate() {
        orgs.push(json!({ "id": rng.uuid(), "name": format!("{}{} Inc.", slug[..1].to_uppercase(), &slug[1..]), "slug": slug }));
        let mut staff = Vec::new();
        for a in 0..AGENTS_PER_ORG {
            let id = rng.uuid();
            let role = match a {
                0 => "admin",
                1..=6 => "agent",
                _ => "viewer",
            };
            if role != "viewer" {
                staff.push(id);
            }
            agents.push(json!({
                "id": id, "org": slug, "name": format!("Agent {a} of {slug}"), "email": format!("agent{a}@{slug}.example"),
                "role": role, "active": a != 6,
            }));
        }
        let mut org_tags = Vec::new();
        for name in TAG_NAMES.iter().take(TAGS_PER_ORG) {
            let id = rng.uuid();
            org_tags.push(id);
            tags.push(json!({ "id": id, "org": slug, "name": name }));
        }
        for t in 0..TICKETS_PER_ORG {
            let id = rng.uuid();
            // Every tenth ticket ties with the one before it.
            let seq = (o * TICKETS_PER_ORG + t) as u64;
            let opened_at = 30 * (seq - u64::from(t % 10 == 9 && t > 0));
            let status = rng.weighted(STATUSES);
            let assignee = (!rng.chance(20)).then(|| *rng.pick(&staff));
            tickets.push(json!({
                "id": id, "org": slug,
                "subject": format!("{} {}", rng.words(1, 6), t),
                "body": rng.words(5, 30),
                "priority": 1 + rng.below(4).min(rng.below(4)),
                "confidential": rng.chance(15),
                "requester_email": format!("customer{}@{slug}-customers.example", rng.below(400)),
                "assignee_id": assignee, "author_id": rng.pick(&staff),
                "status": status,
                "view_count": rng.below(50), "reopen_count": rng.below(3), "version": 1 + rng.below(5),
                "inserted_at": at(opened_at), "updated_at": at(opened_at + rng.below(86_400)),
            }));
            let mut filed = Vec::new();
            for _ in 0..rng.below(4) {
                let tag = *rng.pick(&org_tags);
                if !filed.contains(&tag) {
                    filed.push(tag);
                    ticket_tags.push(json!({ "id": rng.uuid(), "org": slug, "ticket_id": id, "tag_id": tag }));
                }
            }
            opened.push((slug, id, staff.clone(), opened_at));
        }
    }
    // Comments land unevenly: a few tickets get many, some get none.
    let mut comments = Vec::with_capacity(COMMENTS);
    while comments.len() < COMMENTS {
        let skewed = rng.below(opened.len() as u64).min(rng.below(opened.len() as u64)) as usize;
        let (slug, ticket, staff, opened_at) = &opened[skewed];
        let written = opened_at + 60 + rng.below(604_800);
        comments.push(json!({
            "id": rng.uuid(), "org": slug, "ticket_id": ticket, "author_id": rng.pick(staff),
            "body": rng.words(3, 40), "internal": rng.chance(25),
            "inserted_at": at(written), "updated_at": at(written),
        }));
    }
    json!({ "orgs": orgs, "agents": agents, "tags": tags, "tickets": tickets, "comments": comments, "ticket_tags": ticket_tags })
}

/// A fixture record as `resource`'s attributes.
fn fields(resource: &ResourceDef, record: &Map<String, Json>) -> FieldMap {
    let mut fields = FieldMap::new();
    for (name, value) in record {
        let Some(attr) = resource.attribute(name) else { continue };
        let value = match (value, attr.ty) {
            (Json::Null, _) => Value::Null,
            (Json::String(s), AttrType::Uuid) => Uuid::parse_str(s).map(Value::Uuid).unwrap_or(Value::Null),
            (Json::String(s), _) => Value::String(s.clone()),
            (Json::Bool(b), _) => Value::Bool(*b),
            (Json::Number(n), _) => n.as_i64().map(Value::Int).unwrap_or(Value::Null),
            (other, _) => Value::from_plain_json(other.clone()),
        };
        fields.insert(name.clone(), value);
    }
    fields
}

/// Loads `fixture` through each resource's `seed` action, as an admin of each org.
pub async fn load<D: DataLayer + 'static>(ctx: &Context<D>, fixture: &Json) -> Result<()> {
    let records = |key: &str| fixture[key].as_array().cloned().unwrap_or_default();
    let admin = ash_core::Actor::new(Uuid::nil()).with_role("admin");
    let ctx = ctx.with_actor(admin);
    seed::<Org, D>(&ctx, &records("orgs")).await?;
    for org in ORGS {
        let tenant = ctx.clone().with_tenant(*org);
        let of_org = |key: &str| -> Vec<Json> {
            records(key).into_iter().filter(|r| r["org"] == *org).collect()
        };
        seed::<Agent, D>(&tenant, &of_org("agents")).await?;
        seed::<Tag, D>(&tenant, &of_org("tags")).await?;
        seed::<Ticket, D>(&tenant, &of_org("tickets")).await?;
        seed::<Comment, D>(&tenant, &of_org("comments")).await?;
        seed::<TicketTag, D>(&tenant, &of_org("ticket_tags")).await?;
    }
    Ok(())
}

async fn seed<R: Resource, D: DataLayer + 'static>(ctx: &Context<D>, records: &[Json]) -> Result<()> {
    let rows: Vec<FieldMap> = records
        .iter()
        .filter_map(Json::as_object)
        .map(|record| fields(&R::DEF, record))
        .collect();
    let opts = ash_core::BulkCreateOptions::new()
        .batch_size(1_000)
        .return_records(false)
        .notify(false);
    let result = ash_core::bulk_create::<R, D, _, _>(ctx, "seed", rows, opts).await?;
    if result.error_count > 0 {
        return Err(ash_core::Error::Invalid(format!("seeding {}: {:?}", R::DEF.name, result.errors.first())));
    }
    Ok(())
}
