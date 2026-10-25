use ash_core::{
    AttrType, CalculationDef, CompiledQuery, DataLayerKind, Expr, FieldMap, FragmentPart as P,
    Resource, SqlFragment, resource,
};
use ash_sql::{PostgresDialect, QueryCompiler, SqliteDialect};
use uuid::Uuid;

resource! {
    Row {
        table "fragment_rows";
        attributes { id: Uuid [pk]; title: String; }
        actions { read read { primary; } }
    }
}
const FRAGMENT: SqlFragment = SqlFragment::new(
    &[DataLayerKind::Postgres, DataLayerKind::Sqlite],
    &[
        P::Sql("lower("),
        P::Field("title"),
        P::Sql(") = "),
        P::Param {
            name: "term",
            ty: AttrType::String,
        },
    ],
);

#[test]
fn fragment_fields_use_the_current_alias_and_values_remain_bound() {
    let filter = FRAGMENT
        .bind(ash_core::fields! {"term" => "'); DROP TABLE users; --"})
        .into_filter();
    for (sql, params) in [
        {
            let mut compiler = QueryCompiler::new(&PostgresDialect);
            (
                compiler
                    .compile_filter_scoped(&Row::DEF, &filter, Some("\"related_alias\""))
                    .unwrap(),
                compiler.params,
            )
        },
        {
            let mut compiler = QueryCompiler::new(&SqliteDialect);
            (
                compiler
                    .compile_filter_scoped(&Row::DEF, &filter, Some("\"related_alias\""))
                    .unwrap(),
                compiler.params,
            )
        },
    ] {
        assert!(sql.contains("\"related_alias\".\"title\""));
        assert!(!sql.contains("DROP TABLE"));
        assert_eq!(
            params[0].value,
            ash_core::Value::String("'); DROP TABLE users; --".into())
        );
    }
}

#[test]
fn fragment_calculations_keep_correlated_field_aliases() {
    static CALCULATIONS: &[CalculationDef] = &[CalculationDef::new(
        "match_title",
        AttrType::Boolean,
        Expr::Fragment(SqlFragment::new(
            &[DataLayerKind::Postgres],
            &[P::Sql("length("), P::Field("title"), P::Sql(") > 0")],
        )),
    )];
    let resource = ash_core::ResourceDef {
        calculations: CALCULATIONS,
        ..Row::DEF
    };
    let mut compiler = QueryCompiler::new(&PostgresDialect);
    let sql = compiler
        .compile_operand_scoped(&resource, "match_title", Some("\"lateral_row\""))
        .unwrap();
    assert!(sql.contains("\"lateral_row\".\"title\""), "{sql}");
}

#[test]
fn fragment_parameters_and_fields_are_validated_during_compilation() {
    let mut compiler = QueryCompiler::new(&PostgresDialect);
    for parameters in [FieldMap::new(), ash_core::fields! {"term" => 12}] {
        assert!(
            compiler
                .compile_select(
                    &Row::DEF,
                    &CompiledQuery {
                        filter: Some(FRAGMENT.bind(parameters).into_filter()),
                        ..Default::default()
                    }
                )
                .is_err()
        );
    }
    let bad = SqlFragment::new(&[DataLayerKind::Postgres], &[P::Field("missing")]);
    assert!(
        compiler
            .compile_select(
                &Row::DEF,
                &CompiledQuery {
                    filter: Some(bad.bind(FieldMap::new()).into_filter()),
                    ..Default::default()
                }
            )
            .is_err()
    );
}

#[test]
fn fragments_keep_schema_tenancy_and_lateral_per_key_queries() {
    let resource = ash_core::ResourceDef {
        multitenancy: Some(ash_core::MultitenancyDef::context()),
        ..Row::DEF
    };
    let query = CompiledQuery {
        filter: Some(
            FRAGMENT
                .bind(ash_core::fields! {"term" => "rust"})
                .into_filter(),
        ),
        tenant: Some("tenant_a".into()),
        ..Default::default()
    };
    let sql = QueryCompiler::new(&PostgresDialect)
        .compile_select_per_key(
            &resource,
            &query,
            &ash_core::PerKey::Attribute("id"),
            vec![Uuid::nil().into()],
        )
        .unwrap();
    assert!(sql.sql.contains("CROSS JOIN LATERAL"));
    assert!(
        sql.sql.contains("FROM \"tenant_a\".\"fragment_rows\""),
        "{}",
        sql.sql
    );
    assert!(sql.sql.contains("\"id\" = \"__ash_k\".\"key\""));
    assert!(
        sql.params
            .iter()
            .any(|param| param.value == ash_core::Value::String("rust".into()))
    );
}
