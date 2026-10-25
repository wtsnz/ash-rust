#[path = "support/fixtures.rs"]
mod scoped_fixtures;
use scoped_fixtures::Fixtures;

#[path = "support/derived_values.rs"]
mod support;

use ash_core::{
    ClientExpr, CompiledQuery, DataLayer, FieldMap, Filter, ReadRequest, Resource, Runtime,
    SchemaSupport, Sort, Value,
};
use support::*;

async fn visibility<D: DataLayer>(runtime: Runtime<D>) {
    let id = seed(&runtime).await;
    for (ctx, privileged) in [
        (runtime.bind(Some(member()), None), false),
        (runtime.bind(Some(admin()), None), true),
        (runtime.system().bootstrap(), true),
    ] {
        let rows = Employee::query(&ctx)
            .sort(Employee::name)
            .calc(employee::fields::salary_x2)
            .calc(employee::fields::ssn_length)
            .calc(employee::fields::fallback)
            .calc(employee::fields::conditional)
            .calc(employee::fields::restricted)
            .calc(employee::fields::ssn_last_4)
            .calc(employee::fields::private_last_4)
            .calc(employee::fields::restricted_last_4)
            .calc(employee::fields::declared_public)
            .calc(employee::fields::profile_secret)
            .calc(employee::fields::profile_suffix)
            .all()
            .await
            .unwrap();
        let hidden = rows.iter().find(|row| row.name == "hidden").unwrap();
        assert_eq!(hidden.salary_x2, privileged.then_some(1000));
        assert_eq!(hidden.ssn_length, privileged.then_some(11));
        assert_eq!(
            hidden.fallback.as_deref(),
            privileged.then_some("111-11-0001")
        );
        assert_eq!(hidden.conditional.as_deref(), privileged.then_some("paid"));
        assert_eq!(hidden.restricted.as_deref(), privileged.then_some("hidden"));
        assert_eq!(
            hidden.private_last_4.as_deref(),
            privileged.then_some("0001")
        );
        assert_eq!(
            hidden.restricted_last_4.as_deref(),
            privileged.then_some("0001")
        );
        assert_eq!(hidden.declared_public, privileged.then_some(7));
        assert_eq!(
            hidden.profile_secret.as_deref(),
            privileged.then_some("embedded-private")
        );
        assert_eq!(hidden.profile_suffix.as_deref(), Some("vate"));
        assert_eq!(hidden.ssn_last_4.as_deref(), Some("0001"));
        let public = rows.iter().find(|row| row.name == "small").unwrap();
        assert_eq!(public.salary_x2, Some(200));

        let value = ctx
            .present(
                &Employee::DEF,
                ash_core::fields! {
                    "public" => false, "salary_x2" => 1000, "ssn_last_4" => "0001"
                }
                .into(),
                None,
            )
            .unwrap();
        assert_eq!(value.is_forbidden("salary_x2"), !privileged);
        assert!(!value.is_forbidden("ssn_last_4"));
        assert!(
            !serde_json::to_string(&value)
                .unwrap()
                .contains("__ash_projected_")
        );

        let dept = Department::query(&ctx)
            .filter(Department::id.eq(id))
            .aggregate(department::fields::total_salary)
            .aggregate(department::fields::top_ssn)
            .aggregate(department::fields::first_salary)
            .aggregate(department::fields::employee_count)
            .aggregate(department::fields::has_employees)
            .aggregate(department::fields::restricted_count)
            .aggregate(department::fields::linked_salary)
            .aggregate(department::fields::linked_ssn)
            .all()
            .await
            .unwrap()
            .pop()
            .unwrap();
        // System bypasses the disabled source row's read policy too.
        let system = ctx.actor().is_none();
        let total = if system {
            1800
        } else if privileged {
            800
        } else {
            300
        };
        assert_eq!(dept.total_salary, Some(total));
        assert_eq!(dept.linked_salary, Some(total));
        assert_eq!(dept.top_ssn.as_deref(), privileged.then_some("111-11-0001"));
        assert_eq!(
            dept.linked_ssn.as_deref(),
            privileged.then_some("111-11-0001")
        );
        assert_eq!(dept.first_salary, privileged.then_some(500));
        assert_eq!(dept.employee_count, Some(if system { 4 } else { 3 }));
        assert_eq!(dept.has_employees, Some(true));
        assert_eq!(
            dept.restricted_count,
            if privileged {
                Some(if system { 4 } else { 3 })
            } else {
                None
            }
        );
    }
    let ctx = runtime.bind(Some(member()), None);
    let client = |filter| ClientExpr::new(filter).into_filter();
    for filter in [
        Filter::eq("fallback", "111-11-0001"),
        !Filter::eq("fallback", "unknown"),
        Filter::gt("salary_x2", 500),
        Filter::eq("restricted", "hidden"),
    ] {
        assert!(
            Employee::query(&ctx)
                .filter(client(filter))
                .all()
                .await
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(
        Employee::query(&ctx)
            .filter(client(Filter::is_nil("fallback")))
            .all()
            .await
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        Employee::query(&ctx)
            .filter(client(Filter::eq("public_ssn_length", 11)))
            .all()
            .await
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        Employee::query(&ctx)
            .action("well_paid")
            .all()
            .await
            .unwrap()
            .len(),
        1,
        "trusted preparations still inspect hidden operands"
    );
    assert_eq!(
        Department::query(&ctx)
            .action("high_total")
            .all()
            .await
            .unwrap()
            .len(),
        1
    );
    for filter in [
        Filter::gt("total_salary", 500),
        Filter::starts_with("top_ssn", "111"),
        !Filter::eq("top_ssn", "unknown"),
        Filter::gt("restricted_count", 0),
    ] {
        assert!(
            Department::query(&ctx)
                .filter(client(filter))
                .all()
                .await
                .unwrap()
                .is_empty()
        );
    }
    assert_eq!(
        Department::query(&ctx)
            .filter(client(Filter::eq("total_salary", 300)))
            .all()
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        Department::query(&ctx)
            .filter(client(Filter::is_nil("top_ssn")))
            .all()
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        Employee::query(&ctx)
            .filter(client(Filter::related(
                "department",
                Filter::gt("total_salary", 500)
            )))
            .all()
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        Employee::query(&ctx)
            .filter(client(Filter::related(
                "department",
                Filter::eq("total_salary", 300)
            )))
            .all()
            .await
            .unwrap()
            .len(),
        3
    );
    let sorted = ash_core::plan_client_sort(
        &Employee::DEF,
        ctx.actor(),
        ClientExpr::new(vec![
            Sort {
                field: "ssn_length".into(),
                descending: true,
                ..Default::default()
            },
            Sort {
                field: "name".into(),
                ..Default::default()
            },
        ]),
    )
    .unwrap();
    let rows = ctx
        .access()
        .read(
            &Employee::DEF,
            "read",
            &FieldMap::new(),
            ReadRequest {
                sort: sorted,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r.get("name").unwrap().as_str().unwrap())
            .collect::<Vec<_>>(),
        ["hidden", "large", "small"]
    );

    let sorts = ash_core::plan_client_sort(
        &Employee::DEF,
        ctx.actor(),
        ClientExpr::new(vec![
            Sort {
                field: "salary_x2".into(),
                descending: true,
                nulls_first: Some(false),
                ..Default::default()
            },
            Sort {
                field: "name".into(),
                ..Default::default()
            },
        ]),
    )
    .unwrap();
    let rows = ctx
        .access()
        .read(
            &Employee::DEF,
            "read",
            &FieldMap::new(),
            ReadRequest {
                sort: sorts,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|r| r.get("name").unwrap().as_str().unwrap())
            .collect::<Vec<_>>(),
        ["large", "small", "hidden"]
    );
    let first = Employee::query(&ctx)
        .action("ranked")
        .page_keyset(1, None, None)
        .await
        .unwrap();
    assert_eq!(first.results[0].name, "hidden");
    assert_eq!(first.results[0].salary_x2, None);
    let second = Employee::query(&ctx)
        .action("ranked")
        .page_keyset(1, first.after.as_deref(), None)
        .await
        .unwrap();
    assert_eq!(second.results[0].name, "large");
    let previous = Employee::query(&ctx)
        .action("ranked")
        .page_keyset(1, None, second.before.as_deref())
        .await
        .unwrap();
    assert_eq!(previous.results[0].name, "hidden");

    // A group with only hidden operands and an empty group both sum to null.
    let empty = Department::create(&ctx).title("empty").await.unwrap();
    let hidden = Department::create(&ctx).title("hidden-only").await.unwrap();
    Employee::create(&ctx)
        .id(uuid::Uuid::from_u128(10))
        .name("only")
        .department_id(hidden.id)
        .salary(Some(50))
        .ssn(Some("private".into()))
        .public(false)
        .enabled(true)
        .await
        .unwrap();
    for id in [empty.id, hidden.id] {
        let row = Department::query(&ctx)
            .filter(Department::id.eq(id))
            .aggregate(department::fields::total_salary)
            .aggregate(department::fields::top_ssn)
            .all()
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(row.total_salary, None);
        assert_eq!(row.top_ssn, None);
    }
    let sorts = ash_core::plan_client_sort(
        &Department::DEF,
        ctx.actor(),
        ClientExpr::new(vec![
            Sort {
                field: "total_salary".into(),
                descending: true,
                ..Default::default()
            },
            Sort {
                field: "title".into(),
                ..Default::default()
            },
        ]),
    )
    .unwrap();
    let rows = ctx
        .access()
        .read(
            &Department::DEF,
            "read",
            &FieldMap::new(),
            ReadRequest {
                sort: sorts.clone(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        rows.iter()
            .map(|row| row.get("title").unwrap().as_str().unwrap())
            .collect::<Vec<_>>(),
        ["empty", "hidden-only", "eng"]
    );
    let first = Department::query(&ctx)
        .action("ranked")
        .page_keyset(1, None, None)
        .await
        .unwrap();
    assert_eq!(first.results[0].title, "eng");
    assert_eq!(first.results[0].total_salary, Some(300));
    let second = Department::query(&ctx)
        .action("ranked")
        .page_keyset(1, first.after.as_deref(), None)
        .await
        .unwrap();
    assert_eq!(second.results[0].title, "hidden-only");
    assert_eq!(second.results[0].total_salary, None);
    let previous = Department::query(&ctx)
        .action("ranked")
        .page_keyset(1, None, second.before.as_deref())
        .await
        .unwrap();
    assert_eq!(previous.results[0].title, "eng");
}

async fn hidden_evaluation<D: DataLayer>(runtime: Runtime<D>) {
    let department_id = seed(&runtime).await;
    let ctx = runtime.bind(Some(member()), None);
    let rows = Employee::query(&ctx)
        .calc(employee::fields::guarded_custom)
        .all()
        .await
        .unwrap();
    assert!(rows.iter().all(|row| row.guarded_custom.is_none()));
    assert!(
        Employee::query(&runtime.bind(Some(admin()), None))
            .calc(employee::fields::guarded_custom)
            .all()
            .await
            .unwrap_err()
            .to_string()
            .contains("callback inspected")
    );
    for id in [100, 101] {
        Employee::create(&ctx)
            .id(uuid::Uuid::from_u128(id))
            .name("hidden-max")
            .department_id(department_id)
            .salary(Some(i64::MAX))
            .ssn(None)
            .public(false)
            .enabled(true)
            .await
            .unwrap();
    }
    let rows = Employee::query(&ctx)
        .filter(Employee::name.eq("hidden-max"))
        .calc(employee::fields::salary_x2)
        .all()
        .await
        .unwrap();
    assert!(rows.iter().all(|row| row.salary_x2.is_none()));
    let rows = Department::query(&ctx)
        .aggregate(department::fields::total_salary)
        .all()
        .await
        .unwrap();
    assert_eq!(rows[0].total_salary, Some(300));
    let department = Department::create(&ctx)
        .title("restricted-overflow")
        .await
        .unwrap();
    for id in [102, 103] {
        Employee::create(&ctx)
            .id(uuid::Uuid::from_u128(id))
            .name("public-max")
            .department_id(department.id)
            .salary(Some(i64::MAX))
            .ssn(None)
            .public(true)
            .enabled(true)
            .await
            .unwrap();
    }
    let rows = Department::query(&ctx)
        .filter(Department::id.eq(department.id))
        .aggregate(department::fields::restricted_total)
        .all()
        .await
        .unwrap();
    assert_eq!(rows[0].restricted_total, None);
}

#[tokio::test]
async fn memory_hidden_inputs_do_not_execute_callbacks_or_overflow() {
    hidden_evaluation(Runtime::new(ash_memory::Memory::new())).await;
}

#[tokio::test]
async fn sqlite_hidden_inputs_do_not_execute_callbacks_or_overflow() {
    let data = ash_sqlite::Sqlite::memory().await.unwrap();
    data.install_resources(RESOURCES).await.unwrap();
    hidden_evaluation(Runtime::new(data)).await;
}

#[tokio::test]
async fn memory_derived_values_inherit_visibility_and_client_comparisons() {
    visibility(Runtime::new(ash_memory::Memory::new())).await;
}
#[tokio::test]
async fn sqlite_derived_values_inherit_visibility_and_client_comparisons() {
    let data = ash_sqlite::Sqlite::memory().await.unwrap();
    data.install_resources(RESOURCES).await.unwrap();
    visibility(Runtime::new(data)).await;
}

#[tokio::test]
async fn custom_calculations_require_declared_dependencies() {
    static CALCS: &[ash_core::CalculationDef] = &[ash_core::CalculationDef::new(
        "opaque",
        ash_core::AttrType::String,
        ash_core::Expr::Custom(last_four),
    )];
    let resource = ash_core::ResourceDef {
        calculations: CALCS,
        ..Employee::DEF
    };
    let query = CompiledQuery {
        calculations: vec!["opaque".into()],
        ..Default::default()
    };
    let data = ash_memory::Memory::new();
    assert!(
        data.fixture_run_query(&resource, &query)
            .await
            .unwrap_err()
            .to_string()
            .contains("must declare dependencies")
    );
    data.fixture_create(
        &resource,
        None,
        Value::Uuid(uuid::Uuid::nil()),
        ash_core::fields! {"id" => uuid::Uuid::nil(), "name" => "fixture", "department_id" => uuid::Uuid::nil(), "public" => true, "enabled" => true},
    )
    .await
    .unwrap();
    assert!(
        data.fixture_run_query(&resource, &query)
            .await
            .unwrap_err()
            .to_string()
            .contains("must declare dependencies")
    );
}

#[tokio::test]
async fn handwritten_calculations_require_stored_dependencies() {
    static CALCULATIONS: &[ash_core::CalculationDef] = &[ash_core::CalculationDef::new(
        "copy_total",
        ash_core::AttrType::Integer,
        ash_core::Expr::Field("total_salary"),
    )];
    let resource = ash_core::ResourceDef {
        calculations: CALCULATIONS,
        ..Department::DEF
    };
    let query = CompiledQuery {
        calculations: vec!["copy_total".into()],
        ..Default::default()
    };
    let memory = ash_memory::Memory::new();
    let sqlite = ash_sqlite::Sqlite::memory().await.unwrap();
    for message in [
        memory
            .fixture_run_query(&resource, &query)
            .await
            .unwrap_err()
            .to_string(),
        sqlite
            .fixture_run_query(&resource, &query)
            .await
            .unwrap_err()
            .to_string(),
    ] {
        assert!(message.contains("must be a stored attribute"), "{message}");
    }
    assert!(
        Runtime::new(memory)
            .bootstrap()
            .present(
                &resource,
                ash_core::fields! {"copy_total" => 800}.into(),
                None
            )
            .is_err()
    );
}
