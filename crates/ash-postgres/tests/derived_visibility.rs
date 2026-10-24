#[path = "../../ash-core/tests/support/derived_values.rs"]
mod support;

use ash_core::{ClientExpr, Filter, Runtime, TransactionSupport};
use ash_postgres::Postgres;
use support::*;

#[tokio::test]
async fn postgres_projects_derived_values_in_lateral_and_correlated_queries() {
    let url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".into());
    let Ok(data) = Postgres::connect(&url).await else {
        assert!(std::env::var("CI").is_err(), "CI runs Postgres");
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    // Roll back the fixture so repeated runs use the same tables without retaining rows.
    data.install(RESOURCES).await.unwrap();
    let result: ash_core::Result<()> = data
        .transaction(|data| {
            let data = data.clone();
            async move {
                let runtime = Runtime::new(data.clone());
                seed(&runtime).await;
                let ctx = runtime.bind(Some(member()), None);
                let row = Department::query(&ctx)
                    .aggregate(department::fields::total_salary)
                    .aggregate(department::fields::top_ssn)
                    .aggregate(department::fields::first_salary)
                    .aggregate(department::fields::linked_salary)
                    .aggregate(department::fields::linked_ssn)
                    .aggregate(department::fields::employee_count)
                    .all()
                    .await
                    .unwrap()
                    .pop()
                    .unwrap();
                assert_eq!(row.total_salary, Some(300));
                assert_eq!(row.linked_salary, Some(300));
                assert_eq!(row.first_salary, None);
                assert_eq!(row.top_ssn, None);
                assert_eq!(row.linked_ssn, None);
                assert_eq!(row.employee_count, Some(3));
                for filter in [
                    Filter::gt("total_salary", 500),
                    Filter::starts_with("top_ssn", "111"),
                ] {
                    assert!(
                        Department::query(&ctx)
                            .filter(ClientExpr::new(filter).into_filter())
                            .all()
                            .await
                            .unwrap()
                            .is_empty()
                    );
                }
                let row = Employee::query(&ctx)
                    .filter(Employee::name.eq("hidden"))
                    .calc(employee::fields::salary_x2)
                    .calc(employee::fields::ssn_last_4)
                    .calc(employee::fields::fallback)
                    .calc(employee::fields::guarded_custom)
                    .calc(employee::fields::profile_secret)
                    .calc(employee::fields::profile_suffix)
                    .all()
                    .await
                    .unwrap()
                    .pop()
                    .unwrap();
                assert_eq!(row.salary_x2, None);
                assert_eq!(row.fallback, None);
                assert_eq!(row.guarded_custom, None);
                assert_eq!(row.profile_secret, None);
                assert_eq!(row.profile_suffix.as_deref(), Some("vate"));
                assert_eq!(row.ssn_last_4.as_deref(), Some("0001"));
                assert_eq!(
                    Department::query(&ctx)
                        .action("high_total")
                        .all()
                        .await
                        .unwrap()
                        .len(),
                    1
                );
                let department_id = Department::query(&ctx).all().await.unwrap()[0].id;
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
                Err(ash_core::Error::Invalid("rollback derived fixture".into()))
            }
        })
        .await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("rollback derived fixture")
    );
}
