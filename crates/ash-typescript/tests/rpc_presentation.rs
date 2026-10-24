use ash_core::{Actor, Runtime, TransactionSupport, resource};
use ash_memory::Memory;
use serde_json::json;
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

use ash_typescript::rpc::Rpc;

async fn check_results<D: TransactionSupport + Clone + 'static>(runtime: Runtime<D>) {
    let rpc = Rpc::new()
        .action::<Employee>("read", "read")
        .action::<Employee>("create", "create")
        .action::<Employee>("update", "update")
        .action::<Employee>("destroy", "destroy");
    for (ctx, visible) in [
        (runtime.bind(Some(admin()), None), true),
        (runtime.bind(Some(member()), None), false),
        (runtime.system().bootstrap(), true),
    ] {
        let fields = json!(["id", "name", "salary", "ssn", {"profile": ["bio", "secret"]}]);
        let answer = rpc.run(&ctx, &json!({"action":"create", "fields":fields, "input":{"name":"employee", "salary":999,"ssn":"number", "profile":{"bio":"visible", "secret":"embedded secret"}}})).await;
        assert_eq!(answer["success"], true, "{answer}");
        let created = answer["data"].clone();
        let id = created["id"].clone();
        for (action, input) in [
            ("read", json!({})),
            ("update", json!({"name":"renamed"})),
            ("destroy", json!({})),
        ] {
            let answer = rpc
                .run(
                    &ctx,
                    &json!({"action":action,"fields":fields,"identity":id,"input":input}),
                )
                .await;
            assert_eq!(answer["success"], true, "{answer}");
            let record = if action == "read" {
                &answer["data"][0]
            } else {
                &answer["data"]
            };
            for record in [record, &created] {
                assert_eq!(record["salary"].is_null(), !visible, "{action}: {record}");
                assert_eq!(record["ssn"].is_null(), !visible, "{action}: {record}");
                assert_eq!(
                    record["profile"]["secret"].is_null(),
                    !visible,
                    "{action}: {record}"
                );
                assert_eq!(record["profile"]["bio"], "visible");
                assert_eq!(record["profile"].as_object().unwrap().len(), 2);
            }
        }
    }
}

#[tokio::test]
async fn rpc_reads_and_write_results_project_embedded_and_top_level_fields_memory() {
    check_results(Runtime::new(Memory::new())).await;
}

#[tokio::test]
async fn generic_resource_and_custom_runner_results_are_presented_with_declared_metadata() {
    use ash_core::Resource;
    let ctx = Runtime::new(Memory::new()).bind(Some(member()), None);
    let rpc = Rpc::new()
        .action::<Employee>("snapshot", "profile_snapshot")
        .generic::<Employee, _, _>("custom", "profile_snapshot", |_, _| async {
            let profile = profile::Profile::build_create()
                .bio("custom")
                .secret("hidden")
                .build()?;
            Ok(ash_core::Value::Map(profile.to_fields()))
        });
    let answer = rpc.run(&ctx, &json!({"action":"snapshot"})).await;
    assert_eq!(answer["success"], true, "{answer}");
    assert_eq!(answer["data"]["secret"], "allowed");
    assert!(answer["data"]["gate"].is_null());
    let answer = rpc.run(&ctx, &json!({"action":"custom"})).await;
    assert_eq!(answer["success"], true, "{answer}");
    assert_eq!(answer["data"]["bio"], "custom");
    assert!(answer["data"]["secret"].is_null());
}
