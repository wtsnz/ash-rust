use ash_core::{Actor, DataLayer, Resource, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

fn admin() -> Actor {
    Actor::new(Uuid::new_v4()).with_role("admin")
}
fn member() -> Actor {
    Actor::new(Uuid::new_v4()).with_role("member")
}

pub mod profile {
    use super::*;
    resource! {
        embedded Profile {
            actor { role: String; }
            attributes {
                bio: String;
                secret: Option<String>;
            }
            field_policies {
                field secret {
                    authorize_if actor_attribute_equals(role, "admin");
                }
            }
            actions {
                create create { primary; accept [bio, secret]; }
            }
        }
    }
}

pub mod employee {
    use super::profile::Profile;
    use super::*;
    resource! {
        Employee {
            table "presentation_employees";
            actor { role: String; }
            attributes {
                id: Uuid [pk];
                name: String;
                dept_id: Option<Uuid>;
                ssn: Option<String>;
                salary: Option<i64>;
                profile: Option<Profile>;
            }
            policies {
                policy always { authorize_if actor_present; }
            }
            field_policies {
                field ssn { authorize_if actor_attribute_equals(role, "admin"); }
                field salary { authorize_if actor_attribute_equals(role, "admin"); }
            }
            actions {
                create create { primary; accept [name, dept_id, ssn, salary, profile]; }
                create hire {
                    accept [name];
                    change set(salary = 4242);
                    change set(ssn = "000-11-2222");
                }
                create intake {
                    accept [name, ssn, salary];
                    persist manual;
                }
                read read { primary; }
                update update { primary; accept [name]; }
                destroy destroy { primary; }
                generic snapshot {
                    returns Employee;
                    run |_input| async move {
                        Employee::build_create().name("generic").salary(123).profile(
                            Profile::build_create().bio("bio").secret("secret").build()?
                        ).build()
                    };
                }
                generic snapshots {
                    returns Vec<Employee>;
                    run |_input| async move {
                        Ok(vec![Employee::build_create().name("generic").salary(123).build()?])
                    };
                }

            }
        }
    }
}

use employee::Employee;
use profile::Profile;

async fn write_results<D: ash_core::TransactionSupport + 'static>(
    base: ash_core::Runtime<D>,
    label: &str,
) {
    for (who, actor) in [("admin", admin()), ("member", member())] {
        let ctx = base.bind(Some(actor), None);
        // Server-set hidden values (the writer never supplied them).
        let hired = Employee::hire(&ctx).name("h").await.unwrap();
        println!(
            "[{label}] {who}: create result salary={:?} ssn={:?}",
            hired.salary, hired.ssn
        );
        let updated = hired
            .clone()
            .update_on(&ctx)
            .name("h2".to_string())
            .await
            .unwrap();
        println!(
            "[{label}] {who}: update result salary={:?} ssn={:?}",
            updated.salary, updated.ssn
        );
        let destroyed = Employee::destroy(&ctx, updated.id).await;
        println!("[{label}] {who}: destroy result = {destroyed:?}");

        let bulk = Employee::bulk_create(
            &ctx,
            (0..2).map(|i| {
                [
                    ("name", ash_core::Value::from(format!("b{i}"))),
                    ("salary", ash_core::Value::from(77_i64)),
                ]
            }),
        )
        .await
        .unwrap();
        let bulk_salaries: Vec<_> = bulk.records.iter().map(|e| e.salary).collect();
        println!("[{label}] {who}: bulk_create result salaries={bulk_salaries:?}");
        let inputs = bulk.records.clone().into_iter().map(|e| {
            let mut input = ash_core::FieldMap::new();
            input.insert("name".into(), ash_core::Value::from("bu"));
            (e, input)
        });
        let bulk_up = Employee::bulk_update(&ctx, "update", inputs).await.unwrap();
        let up_salaries: Vec<_> = bulk_up.records.iter().map(|e| e.salary).collect();
        println!("[{label}] {who}: bulk_update result salaries={up_salaries:?}");
        let ids: Vec<Uuid> = bulk.records.iter().map(|e| e.id).collect();
        let bulk_del = Employee::bulk_destroy(&ctx, &ids).await.unwrap();
        let del_salaries: Vec<_> = bulk_del.records.iter().map(|e| e.salary).collect();
        println!("[{label}] {who}: bulk_destroy result salaries={del_salaries:?}");

        if who == "member" {
            assert_eq!(hired.salary, None, "create result leaks hidden salary");
            assert_eq!(hired.ssn, None, "create result leaks hidden ssn");
            assert_eq!(updated.salary, None, "update result leaks hidden salary");
            assert!(
                bulk_salaries.iter().all(Option::is_none),
                "bulk_create leaks"
            );
            assert!(up_salaries.iter().all(Option::is_none), "bulk_update leaks");
            assert!(
                del_salaries.iter().all(Option::is_none),
                "bulk_destroy leaks"
            );
        } else {
            assert_eq!(hired.salary, Some(4242));
        }
    }
}

#[tokio::test]
async fn write_results_project_top_level_fields_memory() {
    write_results(ash_core::Runtime::new(Memory::new()), "memory").await;
}

#[tokio::test]
async fn write_results_project_top_level_fields_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Employee::DEF]).await.unwrap();
    write_results(ash_core::Runtime::new(sqlite), "sqlite").await;
}

#[tokio::test]
async fn manual_create_result_redacts_hidden_fields() {
    let base = ash_core::Runtime::new(Memory::new());
    for (who, actor) in [("admin", admin()), ("member", member())] {
        let ctx = base.bind(Some(actor), None);
        let made = Employee::intake(&ctx)
            .name("m")
            .ssn(Some("999-99-9999".into()))
            .salary(Some(1))
            .persist(|_ctx, mut e| async move {
                // A manual persist that fills in a server-side value.
                e.insert("salary".into(), ash_core::Value::Int(555_000));
                Ok(e)
            })
            .await
            .unwrap();
        println!(
            "[manual] {who}: manual create result salary={:?} ssn={:?}",
            made.salary, made.ssn
        );
        if who == "member" {
            assert_eq!(
                made.salary, None,
                "manual create result leaks hidden salary"
            );
        }
    }
}

async fn embedded_field_policy<D: DataLayer>(base: ash_core::Runtime<D>, label: &str) {
    for (who, actor) in [("admin", admin()), ("member", member())] {
        let ctx = base.bind(Some(actor), None);
        let profile = Profile::build_create()
            .bio("hi")
            .secret(Some("embedded-secret".into()))
            .build()
            .unwrap();
        let created = Employee::create(&ctx)
            .name("e")
            .profile(Some(profile))
            .await
            .unwrap();
        let read = Employee::get(&ctx, created.id).await.unwrap();
        println!(
            "[{label}] {who}: embedded profile.secret on create result={:?}, on read={:?}",
            created.profile.as_ref().and_then(|p| p.secret.clone()),
            read.profile.as_ref().and_then(|p| p.secret.clone())
        );
        if who == "member" {
            assert_eq!(
                read.profile.as_ref().and_then(|p| p.secret.clone()),
                None,
                "read leaks embedded field-policied value"
            );
            assert_eq!(
                created.profile.as_ref().and_then(|p| p.secret.clone()),
                None,
                "create result leaks embedded field-policied value"
            );
        }
    }
}

#[tokio::test]
async fn embedded_fields_are_projected_on_reads_and_writes_memory() {
    embedded_field_policy(ash_core::Runtime::new(Memory::new()), "memory").await;
}

#[tokio::test]
async fn embedded_fields_are_projected_on_reads_and_writes_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Employee::DEF]).await.unwrap();
    embedded_field_policy(ash_core::Runtime::new(sqlite), "sqlite").await;
}

#[derive(Clone, Debug, PartialEq, ash_core::AshTypedMap)]
pub struct Envelope {
    pub profile: Profile,
}

#[derive(Clone, Debug, PartialEq, ash_core::AshUnion)]
pub enum Choice {
    Profile(Profile),
    Text(String),
}

#[tokio::test]
async fn nested_arrays_maps_and_union_members_are_projected_for_the_reader() {
    use ash_core::AshType;
    let runtime = ash_core::Runtime::new(Memory::new());
    let profile = Profile::build_create()
        .bio("visible")
        .secret("nested secret")
        .build()
        .unwrap();
    let value = vec![Envelope {
        profile: profile.clone(),
    }];
    let union = Choice::Profile(profile);
    for (ctx, visible) in [
        (runtime.bind(Some(member()), None), false),
        (runtime.bind(Some(admin()), None), true),
        (runtime.system().bootstrap(), true),
    ] {
        let projected = ctx
            .present_value(Vec::<Envelope>::ATTR_TYPE, value.to_value())
            .unwrap()
            .to_value();
        let profiles = Vec::<Envelope>::from_value(&projected).unwrap();
        assert_eq!(profiles[0].profile.secret.is_some(), visible);
        let projected = ctx
            .present_value(Choice::ATTR_TYPE, union.to_value())
            .unwrap()
            .to_value();
        let Choice::Profile(profile) = Choice::from_value(&projected).unwrap() else {
            panic!("profile member")
        };
        assert_eq!(profile.secret.is_some(), visible);
    }
}

#[tokio::test]
async fn system_scopes_keep_nested_fields_visible_on_reads_and_writes() {
    let runtime = ash_core::Runtime::new(Memory::new());
    let ctx = runtime.system().bootstrap();
    let profile = Profile::build_create()
        .bio("bio")
        .secret("secret")
        .build()
        .unwrap();
    let employee = Employee::create(&ctx)
        .name("system")
        .salary(42)
        .profile(profile)
        .await
        .unwrap();
    assert_eq!(employee.salary, Some(42));
    assert_eq!(
        employee.profile.as_ref().unwrap().secret.as_deref(),
        Some("secret")
    );
    let employee = Employee::get(&ctx, employee.id).await.unwrap();
    assert_eq!(employee.profile.unwrap().secret.as_deref(), Some("secret"));
}

#[tokio::test]
async fn hooks_inspect_raw_records_and_transaction_hooks_receive_projected_records() {
    let runtime = ash_core::Runtime::new(Memory::new());
    let ctx = runtime.bind(Some(member()), None);
    let record = Employee::hire(&ctx)
        .name("hooked")
        .after_action(|raw| {
            assert_eq!(
                raw.get("salary").and_then(ash_core::Value::as_int),
                Some(4242)
            );
            raw.insert("salary".into(), ash_core::Value::Int(9000));
            Ok(())
        })
        .after_transaction(|result| {
            assert_eq!(result.unwrap().salary, None);
        })
        .await
        .unwrap();
    assert_eq!(record.salary, None);
}

#[tokio::test]
async fn generic_resource_and_array_results_pass_through_presentation() {
    let runtime = ash_core::Runtime::new(Memory::new());
    for (ctx, visible) in [
        (runtime.bind(Some(member()), None), false),
        (runtime.bind(Some(admin()), None), true),
        (runtime.system().bootstrap(), true),
    ] {
        let record = Employee::snapshot(&ctx).await.unwrap();
        assert_eq!(record.salary.is_some(), visible);
        assert_eq!(record.profile.unwrap().secret.is_some(), visible);
        let records = Employee::snapshots(&ctx).await.unwrap();
        assert_eq!(records[0].salary.is_some(), visible);
        let value = Employee::run_generic(&ctx, "snapshot", ash_core::FieldMap::new())
            .await
            .unwrap();
        assert_eq!(
            value.to_value().as_map().unwrap()["salary"].is_null(),
            !visible
        );
    }
}

pub mod conditional_profile {
    use super::*;
    resource! {
        embedded ConditionalProfile {
            attributes { gate: Option<String>; secret: Option<String>; }
            field_policies {
                field gate { forbid_if always; }
                field secret { authorize_if eq(gate, "allow"); }
            }
            actions { create create { primary; accept [gate, secret]; } }
        }
    }
}

#[test]
fn policy_masks_use_intact_operands_before_selection_and_redaction() {
    let ctx = ash_core::Runtime::new(Memory::new()).bootstrap();
    let record = ash_core::FieldMap::from([
        ("gate".into(), "allow".into()),
        ("secret".into(), "visible".into()),
        ("internal".into(), "private".into()),
    ]);
    let view = ctx
        .present(
            &conditional_profile::ConditionalProfile::DEF,
            record.into(),
            Some(&["secret".into()]),
        )
        .unwrap();
    assert!(view.is_forbidden("gate"));
    assert!(!view.is_forbidden("secret"));
    assert_eq!(view.get("secret"), Some(&"visible".into()));
    assert_eq!(view.fields().len(), 1);
    let serialized = serde_json::to_value(&view).unwrap().to_string();
    assert!(!serialized.contains("allow"));
    assert!(!serialized.contains("private"));
}

pub mod assignment {
    use super::*;
    use employee::Employee;
    resource! {
        Assignment {
            table "presentation_assignments";
            attributes { id: Uuid [pk]; employee_id: Option<Uuid>; }
            relationships { belongs_to employee: Employee [fk: employee_id]; }
            field_policies { field employee_id { forbid_if always; } }
            actions {
                create create { primary; accept [employee_id]; }
                read read { primary; }
            }
        }
    }
}

#[tokio::test]
async fn hidden_relationship_keys_load_projected_children_without_entering_serialized_fields() {
    let runtime = ash_core::Runtime::new(Memory::new());
    let admin = runtime.bind(Some(admin()), None);
    let member = runtime.bind(Some(member()), None);
    let profile = Profile::build_create()
        .bio("bio")
        .secret("child secret")
        .build()
        .unwrap();
    let employee = Employee::create(&admin)
        .name("related")
        .salary(77)
        .profile(profile)
        .await
        .unwrap();
    let assignment = assignment::Assignment::create(&member)
        .employee_id(employee.id)
        .await
        .unwrap();
    assert_eq!(assignment.employee_id, None);
    let loaded = assignment::Assignment::query(&member)
        .include(assignment::fields::employee)
        .one()
        .await
        .unwrap();
    let employee = loaded.employee.loaded().unwrap().as_ref().unwrap();
    assert_eq!(employee.salary, None);
    assert_eq!(employee.profile.as_ref().unwrap().secret, None);
    let views = member
        .access()
        .read(
            &assignment::Assignment::DEF,
            "read",
            &ash_core::FieldMap::new(),
            Default::default(),
        )
        .await
        .unwrap();
    let view = &views[0];
    assert!(view.is_forbidden("employee_id"));
    assert!(view.get("employee_id").unwrap().is_null());
    let key = employee.id.to_string();
    assert!(!serde_json::to_string(view).unwrap().contains(&key));
    assert!(!format!("{view:?}").contains(&key));
}

#[test]
fn resource_valued_hook_metadata_is_projected_for_the_views_reader() {
    use ash_core::{AshType, MetadataDef, PresentedValue, put_metadata};
    let ctx = ash_core::Runtime::new(Memory::new()).bind(Some(member()), None);
    let mut fields = ash_core::FieldMap::from([("name".into(), "metadata".into())]);
    let profile = Profile::build_create()
        .bio("visible")
        .secret("metadata secret")
        .build()
        .unwrap();
    put_metadata(&mut fields, "profile", profile.to_value());
    let view = ctx.present(&Employee::DEF, fields.into(), None).unwrap();
    let definition = MetadataDef {
        name: "profile",
        ty: Profile::ATTR_TYPE,
        allow_nil: false,
    };
    let PresentedValue::Record(profile) = view.metadata(&definition).unwrap().unwrap() else {
        panic!("profile")
    };
    assert!(profile.is_forbidden("secret"));
    assert!(profile.get("secret").unwrap().is_null());
    assert_eq!(profile.get("bio"), Some(&"visible".into()));
    assert!(
        !serde_json::to_string(&view)
            .unwrap()
            .contains("metadata secret")
    );
}
