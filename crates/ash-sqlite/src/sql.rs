use ash_core::{
    AggregateDef, AttrType, CompiledQuery, Error, FieldMap, IdentityDef, ResourceDef, Result, Value,
};
use ash_sql::{
    CompiledSql, QueryCompiler, SqliteDialect, column as sql_column, ident as sql_ident,
};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;

pub fn ident(name: &str) -> Result<String> {
    sql_ident(&SqliteDialect, name)
}

pub fn column(resource: &ResourceDef, field: &str) -> Result<String> {
    sql_column(&SqliteDialect, resource, field)
}

pub fn create_table_sql(resource: &ResourceDef) -> Result<String> {
    QueryCompiler::new(&SqliteDialect).compile_create_table(resource)
}

pub fn create_indexes_sql(resource: &ResourceDef) -> Result<Vec<String>> {
    QueryCompiler::new(&SqliteDialect).compile_create_indexes(resource)
}

pub fn select_query(resource: &ResourceDef, query: &CompiledQuery) -> Result<CompiledSql> {
    let mut compiler = QueryCompiler::new(&SqliteDialect);
    compiler.compile_select(resource, query)
}

pub fn insert_query(resource: &ResourceDef, fields: &FieldMap) -> Result<CompiledSql> {
    let mut compiler = QueryCompiler::new(&SqliteDialect);
    compiler.compile_insert(resource, fields)
}

pub fn update_query(resource: &ResourceDef, id: Uuid, fields: &FieldMap) -> Result<CompiledSql> {
    let mut compiler = QueryCompiler::new(&SqliteDialect);
    compiler.compile_update(resource, id, fields)
}

pub fn delete_query(resource: &ResourceDef, id: Uuid) -> Result<CompiledSql> {
    let mut compiler = QueryCompiler::new(&SqliteDialect);
    compiler.compile_delete(resource, id)
}

pub fn bulk_delete_query(resource: &ResourceDef, ids: &[Uuid]) -> Result<CompiledSql> {
    let mut compiler = QueryCompiler::new(&SqliteDialect);
    compiler.compile_bulk_delete(resource, ids)
}

pub fn upsert_query(
    resource: &ResourceDef,
    fields: &FieldMap,
    identity: &IdentityDef,
    update_fields: &[String],
) -> Result<CompiledSql> {
    let mut compiler = QueryCompiler::new(&SqliteDialect);
    compiler.compile_upsert(resource, fields, identity, update_fields)
}

pub fn row_to_fields(
    row: &SqliteRow,
    resource: &ResourceDef,
    calculations: &[String],
    aggregates: &[String],
) -> Result<FieldMap> {
    let mut map = FieldMap::new();

    for attr in resource.attributes {
        let val = extract_column_value(row, attr.name, &attr.ty)?;
        map.insert(attr.name.to_string(), val);
    }

    for calc_name in calculations {
        let calc = resource.calculation(calc_name).ok_or_else(|| {
            Error::Invalid(format!(
                "unknown calculation `{calc_name}` on {}",
                resource.name
            ))
        })?;
        let val = extract_column_value(row, calc.name, &calc.ty)?;
        map.insert(calc.name.to_string(), val);
    }

    for agg_name in aggregates {
        let agg = resource.aggregate(agg_name).ok_or_else(|| {
            Error::Invalid(format!(
                "unknown aggregate `{agg_name}` on {}",
                resource.name
            ))
        })?;
        let val = extract_aggregate_value(row, agg)?;
        map.insert(agg.name.to_string(), val);
    }

    Ok(map)
}

fn extract_column_value(row: &SqliteRow, col: &str, ty: &AttrType) -> Result<Value> {
    match ty {
        AttrType::Uuid => match optional_text(row, col)? {
            None => Ok(Value::Null),
            Some(text) => Ok(Value::Uuid(
                Uuid::parse_str(&text).map_err(|err| Error::DataLayer(err.to_string()))?,
            )),
        },
        AttrType::String | AttrType::Atom { .. } | AttrType::Date | AttrType::UtcDatetime => {
            match optional_text(row, col)? {
                None => Ok(Value::Null),
                Some(text) => Ok(Value::String(text)),
            }
        }
        AttrType::Decimal => match optional_decimal(row, col)? {
            None => Ok(Value::Null),
            Some(text) => Ok(Value::String(text)),
        },
        AttrType::Binary => match optional_blob(row, col)? {
            None => Ok(Value::Null),
            Some(bytes) => Ok(Value::String(ash_core::Binary::from_bytes(bytes).encode())),
        },
        AttrType::Float => match optional_float(row, col)? {
            None => Ok(Value::Null),
            Some(text) => Ok(Value::String(text)),
        },
        AttrType::Integer => match optional_i64(row, col)? {
            None => Ok(Value::Null),
            Some(int) => Ok(Value::Int(int)),
        },
        AttrType::Boolean => match optional_i64(row, col)? {
            None => Ok(Value::Null),
            Some(0) => Ok(Value::Bool(false)),
            Some(_) => Ok(Value::Bool(true)),
        },
        AttrType::Map => match optional_text(row, col)? {
            None => Ok(Value::Null),
            Some(text) => match serde_json::from_str::<FieldMap>(&text) {
                Ok(m) => Ok(Value::Map(m)),
                Err(_) => Ok(Value::Null),
            },
        },
        AttrType::Array => match optional_text(row, col)? {
            None => Ok(Value::Null),
            Some(text) => match serde_json::from_str::<Vec<Value>>(&text) {
                Ok(a) => Ok(Value::Array(a)),
                Err(_) => Ok(Value::Null),
            },
        },
    }
}

fn extract_aggregate_value(row: &SqliteRow, agg: &AggregateDef) -> Result<Value> {
    match agg.ty {
        AttrType::Integer => match optional_i64(row, agg.name)? {
            None => Ok(Value::Null),
            Some(int) => Ok(Value::Int(int)),
        },
        AttrType::Boolean => match optional_i64(row, agg.name)? {
            None => Ok(Value::Null),
            Some(0) => Ok(Value::Bool(false)),
            Some(_) => Ok(Value::Bool(true)),
        },
        AttrType::String | AttrType::Atom { .. } | AttrType::Date | AttrType::UtcDatetime => {
            match optional_text(row, agg.name)? {
                None => Ok(Value::Null),
                Some(text) => Ok(Value::String(text)),
            }
        }
        AttrType::Decimal => match optional_decimal(row, agg.name)? {
            None => Ok(Value::Null),
            Some(text) => Ok(Value::String(text)),
        },
        AttrType::Binary => match optional_blob(row, agg.name)? {
            None => Ok(Value::Null),
            Some(bytes) => Ok(Value::String(ash_core::Binary::from_bytes(bytes).encode())),
        },
        AttrType::Float => match optional_float(row, agg.name)? {
            None => Ok(Value::Null),
            Some(text) => Ok(Value::String(text)),
        },
        AttrType::Uuid => match optional_text(row, agg.name)? {
            None => Ok(Value::Null),
            Some(text) => Ok(Value::Uuid(
                Uuid::parse_str(&text).map_err(|err| Error::DataLayer(err.to_string()))?,
            )),
        },
        AttrType::Map | AttrType::Array => Ok(Value::Null),
    }
}

fn optional_decimal(row: &SqliteRow, name: &str) -> Result<Option<String>> {
    match row.try_get::<Option<String>, _>(name) {
        Ok(value) => return Ok(value),
        Err(sqlx::Error::ColumnNotFound(_)) => return Ok(None),
        Err(_) => {}
    }
    match row.try_get::<Option<i64>, _>(name) {
        Ok(Some(n)) => return Ok(Some(n.to_string())),
        Ok(None) => return Ok(None),
        Err(sqlx::Error::ColumnNotFound(_)) => return Ok(None),
        Err(_) => {}
    }
    match row.try_get::<Option<f64>, _>(name) {
        Ok(Some(n)) => Ok(Some(format_real(n))),
        Ok(None) => Ok(None),
        Err(sqlx::Error::ColumnNotFound(_)) => Ok(None),
        Err(err) => Err(Error::DataLayer(err.to_string())),
    }
}

fn optional_blob(row: &SqliteRow, name: &str) -> Result<Option<Vec<u8>>> {
    match row.try_get::<Option<Vec<u8>>, _>(name) {
        Ok(value) => Ok(value),
        Err(sqlx::Error::ColumnNotFound(_)) => Ok(None),
        Err(err) => Err(Error::DataLayer(err.to_string())),
    }
}

fn optional_float(row: &SqliteRow, name: &str) -> Result<Option<String>> {
    match row.try_get::<Option<f64>, _>(name) {
        Ok(Some(n)) => return Ok(Some(n.to_string())),
        Ok(None) => return Ok(None),
        Err(sqlx::Error::ColumnNotFound(_)) => return Ok(None),
        Err(_) => {}
    }
    match row.try_get::<Option<i64>, _>(name) {
        Ok(Some(n)) => return Ok(Some(n.to_string())),
        Ok(None) => return Ok(None),
        Err(sqlx::Error::ColumnNotFound(_)) => return Ok(None),
        Err(_) => {}
    }
    optional_text(row, name)
}

fn format_real(n: f64) -> String {
    if !n.is_finite() {
        return n.to_string();
    }
    let text = format!("{n:.12}");
    let trimmed = text.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn optional_text(row: &SqliteRow, name: &str) -> Result<Option<String>> {
    match row.try_get::<Option<String>, _>(name) {
        Ok(value) => Ok(value),
        Err(sqlx::Error::ColumnNotFound(_)) => Ok(None),
        Err(err) => Err(Error::DataLayer(err.to_string())),
    }
}

fn optional_i64(row: &SqliteRow, name: &str) -> Result<Option<i64>> {
    match row.try_get::<Option<i64>, _>(name) {
        Ok(value) => Ok(value),
        Err(sqlx::Error::ColumnNotFound(_)) => Ok(None),
        Err(err) => Err(Error::DataLayer(err.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use ash_core::{ActionDef, AttrType, AttributeDef, CalculationDef, Expr, Filter, ResourceDef};

    use super::*;

    const TICKET: ResourceDef = ResourceDef {
        name: "Ticket",
        table: "tickets",
        attributes: &[
            AttributeDef::uuid_pk("id"),
            AttributeDef::required("subject", AttrType::String),
            AttributeDef::required(
                "status",
                AttrType::Atom {
                    one_of: &["open", "closed"],
                },
            ),
            AttributeDef::optional("representative_id", AttrType::Uuid),
        ],
        relationships: &[],
        actions: &[ActionDef::read("read").primary()],
        policies: &[],
        field_policies: &[],
        calculations: &[CalculationDef::new(
            "subject_length",
            AttrType::Integer,
            Expr::StringLength("subject"),
        )],
        aggregates: &[],
        extensions: &[],
        notifiers: &[],
        identities: &[],
        indexes: &[],
        checks: &[],
        statements: &[],
        embedded: false,
        data_layer: ash_core::DataLayerKind::Sqlite,
        timestamps: None,
        store_type_id: ash_core::default_store_type_id,
        store_name: "DefaultStore",
        multitenancy: None,
    };

    fn sql(filter: Filter) -> String {
        let query = CompiledQuery {
            filter: Some(filter),
            ..CompiledQuery::default()
        };
        select_query(&TICKET, &query).unwrap().sql().into()
    }

    #[test]
    fn eq_and_or_compile_to_where() {
        let compiled = sql(Filter::and([
            Filter::eq("status", "open"),
            Filter::or([
                Filter::is_nil("representative_id"),
                Filter::eq("id", uuid::Uuid::nil()),
            ]),
        ]));
        assert!(compiled.contains("WHERE"));
        assert!(compiled.contains("\"status\" = "));
        assert!(compiled.contains("\"representative_id\" IS NULL"));
        assert!(compiled.contains(" OR "));
        assert!(compiled.contains(" AND "));
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = column(&TICKET, "nope").unwrap_err();
        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn calculation_filter_inlines_length() {
        let compiled = sql(Filter::gt("subject_length", 5_i64));
        assert!(compiled.contains("length(\"subject\")"));
        assert!(compiled.contains(" > "));
        assert!(!compiled.contains("AS \"subject_length\""));
    }

    #[test]
    fn loaded_calculation_is_selected() {
        let query = CompiledQuery {
            calculations: vec!["subject_length".into()],
            ..CompiledQuery::default()
        };
        let compiled: String = select_query(&TICKET, &query).unwrap().sql().into();
        assert!(compiled.contains("length(\"subject\") AS \"subject_length\""));
    }

    #[test]
    fn create_table_uses_text_and_nullability() {
        let ddl = create_table_sql(&TICKET).unwrap();
        assert!(ddl.contains("CREATE TABLE IF NOT EXISTS \"tickets\""));
        assert!(ddl.contains("\"id\" TEXT PRIMARY KEY"));
        assert!(ddl.contains("\"subject\" TEXT NOT NULL"));
        assert!(ddl.contains("\"representative_id\" TEXT"));
        assert!(!ddl.contains("\"representative_id\" TEXT NOT NULL"));
    }

    #[test]
    fn in_list_uses_json_each() {
        let ids = vec![Value::Uuid(uuid::Uuid::nil())];
        let compiled = sql(Filter::in_list("id", ids));
        assert!(compiled.contains("\"id\" IN (SELECT value FROM json_each(?))"));
    }
}
