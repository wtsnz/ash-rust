use ash_core::{Actor, DataLayer, Resource, Runtime, resource};
use uuid::Uuid;

pub fn last_four(fields: &ash_core::FieldMap) -> ash_core::Result<ash_core::Value> {
    assert!(
        !fields.contains_key("salary"),
        "callbacks only receive declared inputs"
    );
    Ok(fields
        .get("ssn")
        .and_then(ash_core::Value::as_str)
        .map(|value| {
            ash_core::Value::String(
                value
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect(),
            )
        })
        .unwrap_or(ash_core::Value::Null))
}

pub fn guarded_callback(fields: &ash_core::FieldMap) -> ash_core::Result<ash_core::Value> {
    if fields.get("ssn").is_some_and(|value| !value.is_null()) {
        return Err(ash_core::Error::Invalid(
            "callback inspected a protected input".into(),
        ));
    }
    Ok(ash_core::Value::Null)
}

pub fn profile_secret(fields: &ash_core::FieldMap) -> ash_core::Result<ash_core::Value> {
    Ok(match fields.get("profile") {
        Some(ash_core::Value::Map(profile)) => profile
            .get("secret")
            .cloned()
            .unwrap_or(ash_core::Value::Null),
        _ => ash_core::Value::Null,
    })
}

pub fn profile_suffix(fields: &ash_core::FieldMap) -> ash_core::Result<ash_core::Value> {
    let secret = profile_secret(fields)?;
    last_four(&ash_core::fields! {"ssn" => secret})
}

pub mod profile {
    use super::*;
    resource! {
        embedded Profile {
            actor { role: String; }
            attributes { bio: String; secret: Option<String>; }
            field_policies { field secret { authorize_if actor_attribute_equals(role, "admin"); } }
            actions { create create { primary; accept [bio, secret]; } }
        }
    }
}

pub mod employee {
    use super::department::Department;
    use super::profile::Profile;
    use super::*;
    resource! {
        Employee {
            table "derived_employees";
            actor { role: String; }
            attributes {
                id: Uuid [pk]; name: String; department_id: Uuid;
                salary: Option<i64>; ssn: Option<String>; public: bool; enabled: bool; profile: Option<Profile>;
            }
            relationships { belongs_to department: Department [fk: department_id]; }
            calculations {
                salary_x2: Option<i64> = coalesce(salary, 0) * 2;
                ssn_length: Option<i64> = string_length(ssn);
                public_ssn_length: Option<i64> [declassify] = string_length(ssn);
                fallback: Option<String> = coalesce(ssn, "absent");
                conditional: Option<String> = if_else(coalesce(salary, 0) > 0, "paid", "unpaid");
                restricted: Option<String> = name;
                ssn_last_4: Option<String> [depends_on [ssn], declassify] = custom(last_four);
                private_last_4: Option<String> [depends_on [ssn]] = custom(last_four);
                restricted_last_4: Option<String> [depends_on [ssn], declassify] = custom(last_four);
                profile_secret: Option<String> [depends_on [profile]] = custom(profile_secret);
                profile_suffix: Option<String> [depends_on [profile], declassify] = custom(profile_suffix);
                guarded_custom: Option<String> [depends_on [ssn]] = custom(guarded_callback);
                declared_public: Option<i64> [depends_on [ssn]] = 7;
            }
            policies {
                policy action_type(read) { authorize_if eq(enabled, true); }
                policy action_type(create) | action_type(update) { authorize_if always; }
            }
            field_policies {
                field salary {
                    authorize_if eq(public, true);
                    authorize_if actor_attribute_equals(role, "admin");
                }
                field ssn { authorize_if actor_attribute_equals(role, "admin"); }
                field salary_x2 { authorize_if always; }
                field restricted { authorize_if actor_attribute_equals(role, "admin"); }
                field restricted_last_4 { authorize_if actor_attribute_equals(role, "admin"); }
            }
            actions {
                create create { primary; accept [id, name, department_id, salary, ssn, public, enabled, profile]; }
                update update { primary; accept [name]; }
                read read { primary; }
                read ranked {
                    pagination keyset: true, required: false;
                    prepare sort(salary_x2, desc);
                }
                read well_paid { prepare filter(salary_x2 > 500); }
            }
        }
    }
}

pub mod assignment {
    use super::*;
    resource! {
        Assignment {
            table "derived_assignments";
            attributes { id: Uuid [pk]; department_id: Uuid; employee_id: Uuid; }
            actions {
                create create { primary; accept [department_id, employee_id]; }
                read read { primary; }
            }
        }
    }
}

pub mod department {
    use super::*;
    use super::{assignment::Assignment, employee::Employee};
    resource! {
        Department {
            table "derived_departments";
            actor { role: String; }
            attributes { id: Uuid [pk]; title: String; }
            relationships {
                has_many employees: Employee [fk: department_id];
                many_to_many assigned: Employee [through: Assignment, source_fk: department_id, dest_fk: employee_id];
            }
            aggregates {
                total_salary: Option<i64> = sum(employees, salary);
                top_ssn: Option<String> = first(employees, ssn);
                first_salary: Option<i64> = first(employees, salary);
                employee_count: Option<i64> = count(employees);
                has_employees: Option<bool> = exists(employees);
                restricted_total: Option<i64> = sum(employees, salary);
                restricted_count: Option<i64> = count(employees);
                linked_salary: Option<i64> = sum(assigned, salary);
                linked_ssn: Option<String> = first(assigned, ssn);
            }
            field_policies {
                field restricted_total { authorize_if actor_attribute_equals(role, "admin"); }
                field restricted_count { authorize_if actor_attribute_equals(role, "admin"); }
            }
            actions {
                create create { primary; accept [title]; }
                update update { primary; accept [title]; }
                read read { primary; }
                read ranked {
                    pagination keyset: true, required: false;
                    prepare filter(total_salary > 0);
                    prepare sort(total_salary, desc);
                }
                read high_total { prepare filter(total_salary > 500); }
            }
        }
    }
}

pub use {assignment::Assignment, department::Department, employee::Employee, profile::Profile};
pub const RESOURCES: &[&ash_core::ResourceDef] =
    &[&Employee::DEF, &Department::DEF, &Assignment::DEF];
pub fn admin() -> Actor {
    Actor::new(Uuid::from_u128(1)).with_role("admin")
}
pub fn member() -> Actor {
    Actor::new(Uuid::from_u128(2)).with_role("member")
}

pub async fn seed<D: DataLayer>(runtime: &Runtime<D>) -> Uuid {
    let ctx = runtime.bind(Some(admin()), None);
    let department = Department::create(&ctx).title("eng").await.unwrap();
    for (id, name, salary, public, enabled) in [
        (1, "hidden", 500, false, true),
        (2, "small", 100, true, true),
        (3, "large", 200, true, true),
        (4, "excluded", 1000, true, false),
    ] {
        let employee = Employee::create(&ctx)
            .id(Uuid::from_u128(id))
            .name(name)
            .department_id(department.id)
            .salary(Some(salary))
            .ssn(Some(format!("111-11-{id:04}")))
            .public(public)
            .profile(Some(
                Profile::build_create()
                    .bio("public")
                    .secret("embedded-private")
                    .build()
                    .unwrap(),
            ))
            .enabled(enabled)
            .await
            .unwrap();
        Assignment::create(&ctx)
            .department_id(department.id)
            .employee_id(employee.id)
            .await
            .unwrap();
    }
    department.id
}
