use ash_core::{Actor, Context, Resource, Runtime, TransactionSupport, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use serde_json::Value as Json;
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
                gate: Option<String>;
            }
            field_policies {
                field gate { forbid_if always; }
                field secret {
                    authorize_if eq(gate, "allow");
                    authorize_if actor_attribute_equals(role, "admin");
                }
            }
            actions {
                create create { primary; accept [bio, secret, gate]; }
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
                generic profile_snapshot {
                    returns Profile;
                    run |_input| async move {
                        Profile::build_create().bio("generic").gate("allow").secret("allowed").build()
                    };
                }

            }
        }
    }
}

use employee::Employee;

use ash_graphql::AshGraphQL;
use async_graphql::Request;

async fn execute<D: TransactionSupport + Clone + 'static>(
    ctx: &Context<D>,
    query: &str,
    visible: bool,
) -> Json {
    let schema = AshGraphQL::from_resources(&[&Employee::DEF])
        .finish::<D>()
        .unwrap();
    let response = schema.execute(Request::new(query).data(ctx.clone())).await;
    if visible {
        assert!(response.errors.is_empty(), "{:?}", response.errors);
    } else {
        assert!(!response.errors.is_empty());
        assert!(
            response
                .errors
                .iter()
                .all(|error| error.message == "forbidden field")
        );
        assert!(response.errors.iter().all(|error| {
            error.extensions.as_ref().and_then(|e| e.get("code"))
                == Some(&async_graphql::Value::from("forbidden_field"))
        }));
    }
    response.data.into_json().unwrap()
}

async fn check_results<D: TransactionSupport + Clone + 'static>(runtime: Runtime<D>) {
    for (ctx, visible) in [
        (runtime.bind(Some(admin()), None), true),
        (runtime.bind(Some(member()), None), false),
        (runtime.system().bootstrap(), true),
    ] {
        let created = execute(&ctx, r#"mutation { createEmployee(input: {name: "new", salary: 999, ssn: "number", profile: {bio: "visible", secret: "embedded secret"}}) { errors { message } result { id salary ssn profile { bio secret } } } }"#, visible).await;
        let created = &created["createEmployee"]["result"];
        let id = created["id"].as_str().unwrap();
        let read = execute(
            &ctx,
            &format!(r#"{{ getEmployee(id: "{id}") {{ salary ssn profile {{ bio secret }} }} }}"#),
            visible,
        )
        .await;
        let update = execute(&ctx, &format!(r#"mutation {{ updateEmployee(id: "{id}", input: {{name: "renamed"}}) {{ errors {{ message }} result {{ salary ssn profile {{ bio secret }} }} }} }}"#), visible).await;
        let destroy = execute(&ctx, &format!(r#"mutation {{ destroyEmployee(id: "{id}") {{ errors {{ message }} result {{ salary ssn profile {{ bio secret }} }} }} }}"#), visible).await;
        for record in [
            created,
            &read["getEmployee"],
            &update["updateEmployee"]["result"],
            &destroy["destroyEmployee"]["result"],
        ] {
            assert_eq!(record["salary"].is_null(), !visible, "{record}");
            assert_eq!(record["ssn"].is_null(), !visible, "{record}");
            assert_eq!(record["profile"]["secret"].is_null(), !visible, "{record}");
            assert_eq!(record["profile"]["bio"], "visible");
        }
    }
}

#[tokio::test]
async fn graphql_reads_and_mutations_preserve_embedded_masks_memory() {
    check_results(Runtime::new(Memory::new())).await;
}

#[tokio::test]
async fn graphql_reads_and_mutations_preserve_embedded_masks_sqlite() {
    let data = Sqlite::memory().await.unwrap();
    data.install(&[&Employee::DEF]).await.unwrap();
    check_results(Runtime::new(data)).await;
}

#[tokio::test]
async fn generic_resource_results_keep_masks_computed_from_intact_operands() {
    let ctx = Runtime::new(Memory::new()).bind(Some(member()), None);
    let schema = AshGraphQL::from_resources(&[&Employee::DEF])
        .query_action::<Employee, Memory>("snapshot", "profile_snapshot")
        .finish::<Memory>()
        .unwrap();
    let response = schema
        .execute(Request::new("{ snapshot { bio gate secret } }").data(ctx))
        .await;
    assert_eq!(response.errors.len(), 1, "{:?}", response.errors);
    assert_eq!(response.errors[0].message, "forbidden field");
    let data = response.data.into_json().unwrap();
    assert_eq!(data["snapshot"]["secret"], "allowed");
    assert!(data["snapshot"]["gate"].is_null());
}
