//! Under attribute multitenancy every write stays in its tenant, as Ash keeps it: an
//! identity is unique within the tenant unless it spans all of them, an upsert finds only
//! the tenant's record, and an update or destroy of another tenant's record, even one in
//! hand, finds nothing. And an action writing more than its own row writes in one
//! transaction where the data layer has them: a managed relationship that fails leaves
//! nothing behind. In every data layer.

use ash_core::{Context, Error, FieldMap, Resource, TransactionSupport, Value, resource};
use ash_memory::Memory;
use ash_postgres::Postgres;
use ash_sqlite::Sqlite;
use uuid::Uuid;

fn no_op(_: &mut FieldMap) -> ash_core::Result<()> {
    Ok(())
}

pub mod note {
    use super::*;

    resource! {
        Note {
            table "guard_notes";

            multitenancy {
                strategy: attribute;
                attribute: org;
            }

            attributes {
                id: Uuid [pk];
                org: String;
                text: String;
                code: String;
            }

            identities {
                // Unique within each tenant, as Ash's identities are by default.
                identity unique_text: [text];
                // Unique across all of them.
                identity unique_code: [code], all_tenants;
            }

            actions {
                create create { primary; accept [text, code]; }
                read read { primary; }
                update edit { primary; accept [text]; require_atomic false; change before_action(no_op); }
                destroy destroy { primary; }
            }
        }
    }
}

pub mod family {
    use super::*;

    resource! {
        Family {
            table "guard_families";

            attributes {
                id: Uuid [pk];
                name: String;
            }

            relationships {
                has_many kids: super::kid::Kid [fk: family_id];
            }

            actions {
                read read { primary; }
                create create {
                    primary;
                    accept [name];
                    argument kids: Vec<FieldMap>;
                    change manage_relationship(kids, create);
                }
            }
        }
    }
}

pub mod kid {
    use super::*;

    resource! {
        Kid {
            table "guard_kids";

            attributes {
                id: Uuid [pk];
                family_id: Uuid;
                label: String;
                allowed: bool;
            }

            relationships {
                belongs_to family: super::family::Family [fk: family_id];
            }

            policies {
                policy action_type(create) { authorize_if eq(allowed, true); }
                policy action_type(read) { authorize_if always; }
            }

            actions {
                create create { primary; accept [label, allowed]; }
                read read { primary; }
            }
        }
    }
}

use family::Family;
use kid::Kid;
use note::Note;

/// Two tenants of their own, as Postgres keeps earlier runs' rows.
fn tenants<D>(ctx: &Context<D>) -> (Context<D>, Context<D>) {
    let run = Uuid::new_v4().simple().to_string();
    (ctx.with_tenant(format!("a-{run}")), ctx.with_tenant(format!("b-{run}")))
}

fn code() -> String {
    Uuid::new_v4().simple().to_string()
}

async fn identities_and_upserts<D: TransactionSupport + Clone + 'static>(ctx: &Context<D>) {
    let (a, b) = tenants(ctx);
    let mine = Note::create(&a).text("same").code(code()).await.unwrap();
    // The same text in another tenant is another record; twice in one tenant, a conflict.
    let theirs = Note::create(&b).text("same").code(code()).await.unwrap();
    assert_ne!(mine.id, theirs.id);
    let again = Note::create(&a).text("same").code(code()).await;
    assert!(matches!(again, Err(Error::IdentityConflict { .. })), "{again:?}");
    // An identity across all tenants conflicts across them.
    let shared = code();
    Note::create(&a).text("one").code(shared.clone()).await.unwrap();
    let across = Note::create(&b).text("two").code(shared).await;
    assert!(matches!(across, Err(Error::IdentityConflict { .. })), "{across:?}");

    // An upsert finds only its own tenant's record.
    let (c, d) = tenants(ctx);
    let first = Note::create(&c).text("up").code(code()).await.unwrap();
    let other = Note::create(&d).text("up").code(code()).upsert_on(Note::unique_text, &["code"]).await.unwrap();
    assert_ne!(first.id, other.id, "{other:?}");
    assert_eq!(other.org, d.tenant.clone().unwrap());
}

async fn writes_stay_in_their_tenant<D: TransactionSupport + Clone + 'static>(ctx: &Context<D>) {
    let (a, b) = tenants(ctx);
    let note = Note::create(&a).text("old").code(code()).await.unwrap();
    let text = || FieldMap::from([("text".to_string(), Value::from("changed"))]);

    // Another tenant can't write it, even holding it.
    let updated = ash_core::update_existing(&b, "edit", note.clone(), text()).await;
    assert!(updated.is_err(), "another tenant updated it: {updated:?}");
    let destroyed = ash_core::destroy_existing(&b, "destroy", note.clone()).await;
    assert!(destroyed.is_err(), "another tenant destroyed it: {destroyed:?}");
    assert_eq!(Note::get(&a, note.id).await.unwrap().text, "old");

    // Its own tenant can.
    ash_core::update_existing(&a, "edit", note.clone(), text()).await.unwrap();
    assert_eq!(Note::get(&a, note.id).await.unwrap().text, "changed");
    ash_core::destroy_existing(&a, "destroy", Note::get(&a, note.id).await.unwrap()).await.unwrap();
    assert!(matches!(Note::get(&a, note.id).await, Err(Error::NotFound)));
}

/// A managed relationship that fails partway: in a transaction, nothing is left; without
/// one (ash-memory, as Ash's ETS layer), the record is undone and the related rows written
/// before the failure stay.
async fn failed_managed_create<D: TransactionSupport + Clone + 'static>(ctx: &Context<D>, transactional: bool) {
    let name = format!("family-{}", code());
    let kid = |allowed| FieldMap::from([("label".to_string(), Value::from("kid")), ("allowed".to_string(), Value::from(allowed))]);
    let created = Family::create(ctx).name(name.clone()).kids(vec![kid(true), kid(false)]).await;
    assert!(created.is_err(), "{created:?}");
    let families = Family::query(ctx).filter(ash_core::Filter::eq("name", name)).all().await.unwrap();
    assert!(families.is_empty(), "{families:?}");
    if transactional {
        let ids: Vec<Value> = families.iter().map(|f| Value::Uuid(f.id)).collect();
        let kids = Kid::query(ctx).all().await.unwrap();
        assert!(kids.iter().all(|kid| !ids.contains(&Value::Uuid(kid.family_id))));
        // Nothing of the failed action's: no kid left without its family.
        let orphans = kids.iter().filter(|kid| kid.label == "kid").count();
        assert_eq!(orphans, 0, "the failed action left {orphans} kids");
    }
}

#[tokio::test]
async fn tenant_guards_in_memory() {
    let ctx = Context::new(Memory::new());
    identities_and_upserts(&ctx).await;
    writes_stay_in_their_tenant(&ctx).await;
    failed_managed_create(&ctx, false).await;
}

#[tokio::test]
async fn tenant_guards_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Note::DEF, &Family::DEF, &Kid::DEF]).await.unwrap();
    let ctx = Context::new(sqlite);
    identities_and_upserts(&ctx).await;
    writes_stay_in_their_tenant(&ctx).await;
    failed_managed_create(&ctx, true).await;
}

#[tokio::test]
async fn tenant_guards_in_postgres() {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".into());
    let Ok(pg) = Postgres::connect(&url).await else {
        assert!(std::env::var("CI").is_err(), "CI runs Postgres");
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    // Built afresh: the identities' indexes changed shape.
    for table in ["guard_kids", "guard_families", "guard_notes"] {
        sqlx::query(&format!("DROP TABLE IF EXISTS {table} CASCADE")).execute(pg.pool().unwrap()).await.unwrap();
    }
    pg.install(&[&Note::DEF, &Family::DEF, &Kid::DEF]).await.unwrap();
    let ctx = Context::new(pg);
    identities_and_upserts(&ctx).await;
    writes_stay_in_their_tenant(&ctx).await;
    failed_managed_create(&ctx, true).await;
}
