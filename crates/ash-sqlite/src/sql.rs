use ash_core::{
    AggregateDef, AggregateFilter, AggregateKind, AttrType, CompiledQuery, Error, Expr, FieldMap,
    Filter, RelKind, ResourceDef, Result, Sort, Value,
};
use sqlx::sqlite::SqliteRow;
use sqlx::{QueryBuilder, Row, Sqlite};

pub fn ident(name: &str) -> Result<String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(Error::Invalid(format!("invalid identifier `{name}`")));
    }
    Ok(format!("\"{name}\""))
}

pub fn column(resource: &ResourceDef, field: &str) -> Result<String> {
    if resource.attribute(field).is_none() {
        return Err(Error::Invalid(format!(
            "unknown field `{field}` on {}",
            resource.name
        )));
    }
    ident(field)
}

pub fn create_table_sql(resource: &ResourceDef) -> Result<String> {
    let table = ident(resource.table_name())?;
    let mut cols = Vec::new();
    for attribute in resource.attributes {
        let name = ident(attribute.name)?;
        let ty = match attribute.ty {
            AttrType::Integer | AttrType::Boolean => "INTEGER",
            AttrType::Uuid
            | AttrType::String
            | AttrType::Atom { .. }
            | AttrType::Map
            | AttrType::Array => "TEXT",
        };
        let mut col = format!("{name} {ty}");
        if attribute.primary_key {
            col.push_str(" PRIMARY KEY");
        }
        if !attribute.allow_nil && !attribute.primary_key {
            col.push_str(" NOT NULL");
        }
        cols.push(col);
    }
    for identity in resource.identities {
        let mut key_cols = Vec::new();
        for key in identity.keys {
            key_cols.push(ident(key)?);
        }
        cols.push(format!("UNIQUE ({})", key_cols.join(", ")));
    }
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {table} ({})",
        cols.join(", ")
    ))
}

pub fn create_indexes_sql(resource: &ResourceDef) -> Result<Vec<String>> {
    let mut stmts = Vec::new();
    let table = ident(resource.table_name())?;
    for identity in resource.identities {
        let idx_name = ident(&format!("idx_{}_{}", resource.table_name(), identity.name))?;
        let mut key_cols = Vec::new();
        for key in identity.keys {
            key_cols.push(ident(key)?);
        }
        stmts.push(format!(
            "CREATE UNIQUE INDEX IF NOT EXISTS {idx_name} ON {table} ({})",
            key_cols.join(", ")
        ));
    }
    Ok(stmts)
}

pub fn select_query<'a>(
    resource: &'a ResourceDef,
    query: &'a CompiledQuery,
) -> Result<QueryBuilder<'a, Sqlite>> {
    let mut qb = QueryBuilder::new("SELECT ");
    let mut first = true;
    for attribute in resource.attributes {
        if !first {
            qb.push(", ");
        }
        first = false;
        qb.push(ident(attribute.name)?);
    }
    for name in &query.calculations {
        let calc = resource.calculation(name).ok_or_else(|| {
            Error::Invalid(format!("unknown calculation `{name}` on {}", resource.name))
        })?;
        qb.push(", ");
        push_expr(&mut qb, resource, &calc.expr)?;
        qb.push(" AS ");
        qb.push(ident(calc.name)?);
    }
    for name in &query.aggregates {
        let agg = resource.aggregate(name).ok_or_else(|| {
            Error::Invalid(format!("unknown aggregate `{name}` on {}", resource.name))
        })?;
        qb.push(", ");
        push_aggregate(&mut qb, resource, agg)?;
        qb.push(" AS ");
        qb.push(ident(agg.name)?);
    }
    qb.push(" FROM ");
    qb.push(ident(resource.table_name())?);

    if let Some(filter) = &query.filter {
        qb.push(" WHERE ");
        push_filter(&mut qb, filter, resource)?;
    }

    push_sort(&mut qb, &query.sort, resource)?;

    if let Some(limit) = query.limit {
        qb.push(" LIMIT ");
        qb.push_bind(i64::try_from(limit).unwrap_or(i64::MAX));
    }
    if let Some(offset) = query.offset {
        qb.push(" OFFSET ");
        qb.push_bind(i64::try_from(offset).unwrap_or(i64::MAX));
    }

    Ok(qb)
}

pub fn insert_query<'a>(
    resource: &'a ResourceDef,
    fields: &'a FieldMap,
) -> Result<QueryBuilder<'a, Sqlite>> {
    let mut qb = QueryBuilder::new("INSERT INTO ");
    qb.push(ident(resource.table_name())?);
    qb.push(" (");
    for (i, attribute) in resource.attributes.iter().enumerate() {
        if i > 0 {
            qb.push(", ");
        }
        qb.push(ident(attribute.name)?);
    }
    qb.push(") VALUES (");
    for (i, attribute) in resource.attributes.iter().enumerate() {
        if i > 0 {
            qb.push(", ");
        }
        match fields.get(attribute.name) {
            None | Some(Value::Null) => {
                qb.push("NULL");
            }
            Some(value) => push_sql_value(&mut qb, value),
        }
    }
    qb.push(")");
    Ok(qb)
}

pub fn upsert_query<'a>(
    resource: &'a ResourceDef,
    fields: &'a FieldMap,
    identity: &'a ash_core::IdentityDef,
    update_fields: &'a [String],
) -> Result<QueryBuilder<'a, Sqlite>> {
    let mut qb = insert_query(resource, fields)?;
    qb.push(" ON CONFLICT (");
    for (i, key) in identity.keys.iter().enumerate() {
        if i > 0 {
            qb.push(", ");
        }
        qb.push(ident(key)?);
    }
    qb.push(") DO UPDATE SET ");
    let to_update: Vec<&str> = if update_fields.is_empty() {
        resource
            .attributes
            .iter()
            .filter(|a| !a.primary_key && !identity.keys.contains(&a.name))
            .map(|a| a.name)
            .collect()
    } else {
        update_fields.iter().map(|s| s.as_str()).collect()
    };

    if to_update.is_empty() {
        qb.push("NOTHING");
    } else {
        for (i, col) in to_update.iter().enumerate() {
            if i > 0 {
                qb.push(", ");
            }
            qb.push(ident(col)?);
            qb.push(" = excluded.");
            qb.push(ident(col)?);
        }
    }
    Ok(qb)
}

pub fn update_query<'a>(
    resource: &'a ResourceDef,
    id: uuid::Uuid,
    fields: &'a FieldMap,
) -> Result<QueryBuilder<'a, Sqlite>> {
    let pk = resource
        .primary_key()
        .ok_or(Error::NoPrimaryKey(resource.name))?;
    let mut qb = QueryBuilder::new("UPDATE ");
    qb.push(ident(resource.table_name())?);
    qb.push(" SET ");
    let mut first = true;
    for attribute in resource.attributes {
        if attribute.primary_key {
            continue;
        }
        let Some(val) = fields.get(attribute.name) else {
            continue;
        };
        if !first {
            qb.push(", ");
        }
        first = false;
        qb.push(ident(attribute.name)?);
        qb.push(" = ");
        match val {
            Value::Null => {
                qb.push("NULL");
            }
            value => push_sql_value(&mut qb, value),
        }
    }
    if first {
        qb.push(ident(pk.name)?);
        qb.push(" = ");
        qb.push(ident(pk.name)?);
    }
    qb.push(" WHERE ");
    qb.push(ident(pk.name)?);
    qb.push(" = ");
    qb.push_bind(id.to_string());

    if let Some(v_attr) = resource.optimistic_lock_attribute()
        && let Some(Value::Int(new_v)) = fields.get(v_attr)
    {
        let expected_v = new_v - 1;
            qb.push(" AND ");
            qb.push(ident(v_attr)?);
            qb.push(" = ");
            qb.push_bind(expected_v);
    }
    Ok(qb)
}

pub fn delete_query<'a>(
    resource: &'a ResourceDef,
    id: uuid::Uuid,
) -> Result<QueryBuilder<'a, Sqlite>> {
    let pk = resource
        .primary_key()
        .ok_or(Error::NoPrimaryKey(resource.name))?;
    let mut qb = QueryBuilder::new("DELETE FROM ");
    qb.push(ident(resource.table_name())?);
    qb.push(" WHERE ");
    qb.push(ident(pk.name)?);
    qb.push(" = ");
    qb.push_bind(id.to_string());
    Ok(qb)
}

pub fn row_to_fields(
    row: &SqliteRow,
    resource: &ResourceDef,
    calculations: &[String],
    aggregates: &[String],
) -> Result<FieldMap> {
    let mut map = FieldMap::new();
    for attribute in resource.attributes {
        let value = match attribute.ty {
            AttrType::Uuid => match optional_text(row, attribute.name)? {
                None => Value::Null,
                Some(text) => Value::Uuid(
                    uuid::Uuid::parse_str(&text)
                        .map_err(|err| Error::DataLayer(err.to_string()))?,
                ),
            },
            AttrType::String | AttrType::Atom { .. } => match optional_text(row, attribute.name)? {
                None => Value::Null,
                Some(text) => Value::String(text),
            },
            AttrType::Integer => match optional_i64(row, attribute.name)? {
                None => Value::Null,
                Some(int) => Value::Int(int),
            },
            AttrType::Boolean => match optional_i64(row, attribute.name)? {
                None => Value::Null,
                Some(0) => Value::Bool(false),
                Some(_) => Value::Bool(true),
            },
            AttrType::Map => match optional_text(row, attribute.name)? {
                None => Value::Null,
                Some(text) => match serde_json::from_str::<FieldMap>(&text) {
                    Ok(m) => Value::Map(m),
                    Err(_) => Value::Null,
                },
            },
            AttrType::Array => match optional_text(row, attribute.name)? {
                None => Value::Null,
                Some(text) => match serde_json::from_str::<Vec<Value>>(&text) {
                    Ok(a) => Value::Array(a),
                    Err(_) => Value::Null,
                },
            },
        };
        map.insert(attribute.name.to_string(), value);
    }
    for name in calculations {
        let calc = resource.calculation(name).ok_or_else(|| {
            Error::Invalid(format!("unknown calculation `{name}` on {}", resource.name))
        })?;
        let value = match calc.ty {
            AttrType::Integer => match optional_i64(row, calc.name)? {
                None => Value::Null,
                Some(int) => Value::Int(int),
            },
            AttrType::Boolean => match optional_i64(row, calc.name)? {
                None => Value::Null,
                Some(0) => Value::Bool(false),
                Some(_) => Value::Bool(true),
            },
            AttrType::String | AttrType::Atom { .. } => match optional_text(row, calc.name)? {
                None => Value::Null,
                Some(text) => Value::String(text),
            },
            AttrType::Uuid => match optional_text(row, calc.name)? {
                None => Value::Null,
                Some(text) => Value::Uuid(
                    uuid::Uuid::parse_str(&text)
                        .map_err(|err| Error::DataLayer(err.to_string()))?,
                ),
            },
            AttrType::Map => match optional_text(row, calc.name)? {
                None => Value::Null,
                Some(text) => match serde_json::from_str::<FieldMap>(&text) {
                    Ok(m) => Value::Map(m),
                    Err(_) => Value::Null,
                },
            },
            AttrType::Array => match optional_text(row, calc.name)? {
                None => Value::Null,
                Some(text) => match serde_json::from_str::<Vec<Value>>(&text) {
                    Ok(a) => Value::Array(a),
                    Err(_) => Value::Null,
                },
            },
        };
        map.insert(calc.name.to_string(), value);
    }
    for name in aggregates {
        let agg = resource.aggregate(name).ok_or_else(|| {
            Error::Invalid(format!("unknown aggregate `{name}` on {}", resource.name))
        })?;
        let value = match agg.ty {
            AttrType::Integer => match optional_i64(row, agg.name)? {
                None => Value::Null,
                Some(int) => Value::Int(int),
            },
            AttrType::Boolean => match optional_i64(row, agg.name)? {
                None => Value::Null,
                Some(0) => Value::Bool(false),
                Some(_) => Value::Bool(true),
            },
            AttrType::String | AttrType::Atom { .. } => match optional_text(row, agg.name)? {
                None => Value::Null,
                Some(text) => Value::String(text),
            },
            AttrType::Uuid => match optional_text(row, agg.name)? {
                None => Value::Null,
                Some(text) => Value::Uuid(
                    uuid::Uuid::parse_str(&text)
                        .map_err(|err| Error::DataLayer(err.to_string()))?,
                ),
            },
            AttrType::Map => match optional_text(row, agg.name)? {
                None => Value::Null,
                Some(text) => match serde_json::from_str::<FieldMap>(&text) {
                    Ok(m) => Value::Map(m),
                    Err(_) => Value::Null,
                },
            },
            AttrType::Array => match optional_text(row, agg.name)? {
                None => Value::Null,
                Some(text) => match serde_json::from_str::<Vec<Value>>(&text) {
                    Ok(a) => Value::Array(a),
                    Err(_) => Value::Null,
                },
            },
        };
        map.insert(agg.name.to_string(), value);
    }
    Ok(map)
}

fn optional_text(row: &SqliteRow, name: &str) -> Result<Option<String>> {
    row.try_get(name)
        .map_err(|err| Error::DataLayer(err.to_string()))
}

fn optional_i64(row: &SqliteRow, name: &str) -> Result<Option<i64>> {
    row.try_get(name)
        .map_err(|err| Error::DataLayer(err.to_string()))
}

fn push_sort(
    qb: &mut QueryBuilder<'_, Sqlite>,
    sorts: &[Sort],
    resource: &ResourceDef,
) -> Result<()> {
    if sorts.is_empty() {
        return Ok(());
    }
    qb.push(" ORDER BY ");
    for (i, sort) in sorts.iter().enumerate() {
        if i > 0 {
            qb.push(", ");
        }
        push_operand(qb, resource, &sort.field)?;
        if sort.descending {
            qb.push(" DESC");
        } else {
            qb.push(" ASC");
        }
    }
    Ok(())
}

fn push_filter(
    qb: &mut QueryBuilder<'_, Sqlite>,
    filter: &Filter,
    resource: &ResourceDef,
) -> Result<()> {
    match filter {
        Filter::True => {
            qb.push("1=1");
        }
        Filter::False => {
            qb.push("0=1");
        }
        Filter::Eq(field, value) if value.is_null() => {
            push_operand(qb, resource, field)?;
            qb.push(" IS NULL");
        }
        Filter::Eq(field, value) => {
            push_operand(qb, resource, field)?;
            qb.push(" = ");
            push_sql_value(qb, value);
        }
        Filter::Ne(field, value) if value.is_null() => {
            push_operand(qb, resource, field)?;
            qb.push(" IS NOT NULL");
        }
        Filter::Ne(field, value) => {
            push_operand(qb, resource, field)?;
            qb.push(" <> ");
            push_sql_value(qb, value);
        }
        Filter::Gt(field, value) => {
            push_operand(qb, resource, field)?;
            qb.push(" > ");
            push_sql_value(qb, value);
        }
        Filter::Gte(field, value) => {
            push_operand(qb, resource, field)?;
            qb.push(" >= ");
            push_sql_value(qb, value);
        }
        Filter::Lt(field, value) => {
            push_operand(qb, resource, field)?;
            qb.push(" < ");
            push_sql_value(qb, value);
        }
        Filter::Lte(field, value) => {
            push_operand(qb, resource, field)?;
            qb.push(" <= ");
            push_sql_value(qb, value);
        }
        Filter::IsNil(field) => {
            push_operand(qb, resource, field)?;
            qb.push(" IS NULL");
        }
        Filter::In(field, values) if values.is_empty() => {
            qb.push("0=1");
        }
        Filter::In(field, values) => {
            push_operand(qb, resource, field)?;
            qb.push(" IN (");
            for (i, value) in values.iter().enumerate() {
                if i > 0 {
                    qb.push(", ");
                }
                push_sql_value(qb, value);
            }
            qb.push(")");
        }
        Filter::And(parts) => {
            qb.push("(");
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    qb.push(" AND ");
                }
                push_filter(qb, part, resource)?;
            }
            qb.push(")");
        }
        Filter::Or(parts) => {
            qb.push("(");
            for (i, part) in parts.iter().enumerate() {
                if i > 0 {
                    qb.push(" OR ");
                }
                push_filter(qb, part, resource)?;
            }
            qb.push(")");
        }
        Filter::Not(inner) => {
            qb.push("NOT (");
            push_filter(qb, inner, resource)?;
            qb.push(")");
        }
    }
    Ok(())
}

fn push_operand(
    qb: &mut QueryBuilder<'_, Sqlite>,
    resource: &ResourceDef,
    field: &str,
) -> Result<()> {
    if resource.attribute(field).is_some() {
        qb.push(column(resource, field)?);
        return Ok(());
    }
    if let Some(calc) = resource.calculation(field) {
        push_expr(qb, resource, &calc.expr)?;
        return Ok(());
    }
    if let Some(agg) = resource.aggregate(field) {
        push_aggregate(qb, resource, agg)?;
        return Ok(());
    }
    Err(Error::Invalid(format!(
        "unknown field `{field}` on {}",
        resource.name
    )))
}

fn push_aggregate(
    qb: &mut QueryBuilder<'_, Sqlite>,
    resource: &ResourceDef,
    agg: &AggregateDef,
) -> Result<()> {
    let rel = resource.relationship(agg.relationship).ok_or_else(|| {
        Error::Invalid(format!(
            "unknown relationship `{}` in aggregate `{}` on {}",
            agg.relationship, agg.name, resource.name
        ))
    })?;
    let dest = (rel.destination)();
    let dest_table = dest.table_name();
    let source_table = resource.table_name();
    let dest_attr = rel.destination_attribute;
    let source_attr = rel.source_attribute;

    if rel.kind == RelKind::ManyToMany {
        let through_fn = rel.through.ok_or_else(|| {
            Error::Invalid(format!(
                "many_to_many relationship `{}` requires through join resource",
                rel.name
            ))
        })?;
        let through_def = through_fn();
        let join_table = through_def.table_name();
        let source_on_join = rel
            .source_attribute_on_join_resource
            .unwrap_or(rel.source_attribute);
        let dest_on_join = rel
            .destination_attribute_on_join_resource
            .unwrap_or(rel.destination_attribute);

        match agg.kind {
            AggregateKind::Count => {
                qb.push("(SELECT COUNT(*) FROM ");
                qb.push(ident(dest_table)?);
                qb.push(" JOIN ");
                qb.push(ident(join_table)?);
                qb.push(" ON ");
                qb.push(ident(dest_table)?);
                qb.push(".");
                qb.push(ident(dest_attr)?);
                qb.push(" = ");
                qb.push(ident(join_table)?);
                qb.push(".");
                qb.push(ident(dest_on_join)?);
                qb.push(" WHERE ");
                qb.push(ident(join_table)?);
                qb.push(".");
                qb.push(ident(source_on_join)?);
                qb.push(" = ");
                qb.push(ident(source_table)?);
                qb.push(".");
                qb.push(ident(source_attr)?);
                push_aggregate_filter(qb, dest, &agg.filter)?;
                qb.push(")");
            }
            AggregateKind::Exists => {
                qb.push("(EXISTS (SELECT 1 FROM ");
                qb.push(ident(dest_table)?);
                qb.push(" JOIN ");
                qb.push(ident(join_table)?);
                qb.push(" ON ");
                qb.push(ident(dest_table)?);
                qb.push(".");
                qb.push(ident(dest_attr)?);
                qb.push(" = ");
                qb.push(ident(join_table)?);
                qb.push(".");
                qb.push(ident(dest_on_join)?);
                qb.push(" WHERE ");
                qb.push(ident(join_table)?);
                qb.push(".");
                qb.push(ident(source_on_join)?);
                qb.push(" = ");
                qb.push(ident(source_table)?);
                qb.push(".");
                qb.push(ident(source_attr)?);
                push_aggregate_filter(qb, dest, &agg.filter)?;
                qb.push("))");
            }
            AggregateKind::First { field } => {
                qb.push("(SELECT ");
                qb.push(ident(dest_table)?);
                qb.push(".");
                qb.push(ident(field)?);
                qb.push(" FROM ");
                qb.push(ident(dest_table)?);
                qb.push(" JOIN ");
                qb.push(ident(join_table)?);
                qb.push(" ON ");
                qb.push(ident(dest_table)?);
                qb.push(".");
                qb.push(ident(dest_attr)?);
                qb.push(" = ");
                qb.push(ident(join_table)?);
                qb.push(".");
                qb.push(ident(dest_on_join)?);
                qb.push(" WHERE ");
                qb.push(ident(join_table)?);
                qb.push(".");
                qb.push(ident(source_on_join)?);
                qb.push(" = ");
                qb.push(ident(source_table)?);
                qb.push(".");
                qb.push(ident(source_attr)?);
                push_aggregate_filter(qb, dest, &agg.filter)?;
                qb.push(" LIMIT 1)");
            }
            AggregateKind::Sum { field } => {
                qb.push("(SELECT COALESCE(SUM(");
                qb.push(ident(dest_table)?);
                qb.push(".");
                qb.push(ident(field)?);
                qb.push("), 0) FROM ");
                qb.push(ident(dest_table)?);
                qb.push(" JOIN ");
                qb.push(ident(join_table)?);
                qb.push(" ON ");
                qb.push(ident(dest_table)?);
                qb.push(".");
                qb.push(ident(dest_attr)?);
                qb.push(" = ");
                qb.push(ident(join_table)?);
                qb.push(".");
                qb.push(ident(dest_on_join)?);
                qb.push(" WHERE ");
                qb.push(ident(join_table)?);
                qb.push(".");
                qb.push(ident(source_on_join)?);
                qb.push(" = ");
                qb.push(ident(source_table)?);
                qb.push(".");
                qb.push(ident(source_attr)?);
                push_aggregate_filter(qb, dest, &agg.filter)?;
                qb.push(")");
            }
        }
        return Ok(());
    }

    match agg.kind {
        AggregateKind::Count => {
            qb.push("(SELECT COUNT(*) FROM ");
            qb.push(ident(dest_table)?);
            qb.push(" WHERE ");
            qb.push(ident(dest_table)?);
            qb.push(".");
            qb.push(ident(dest_attr)?);
            qb.push(" = ");
            qb.push(ident(source_table)?);
            qb.push(".");
            qb.push(ident(source_attr)?);
            push_aggregate_filter(qb, dest, &agg.filter)?;
            qb.push(")");
        }
        AggregateKind::Exists => {
            qb.push("(EXISTS (SELECT 1 FROM ");
            qb.push(ident(dest_table)?);
            qb.push(" WHERE ");
            qb.push(ident(dest_table)?);
            qb.push(".");
            qb.push(ident(dest_attr)?);
            qb.push(" = ");
            qb.push(ident(source_table)?);
            qb.push(".");
            qb.push(ident(source_attr)?);
            push_aggregate_filter(qb, dest, &agg.filter)?;
            qb.push("))");
        }
        AggregateKind::First { field } => {
            qb.push("(SELECT ");
            qb.push(ident(dest_table)?);
            qb.push(".");
            qb.push(ident(field)?);
            qb.push(" FROM ");
            qb.push(ident(dest_table)?);
            qb.push(" WHERE ");
            qb.push(ident(dest_table)?);
            qb.push(".");
            qb.push(ident(dest_attr)?);
            qb.push(" = ");
            qb.push(ident(source_table)?);
            qb.push(".");
            qb.push(ident(source_attr)?);
            push_aggregate_filter(qb, dest, &agg.filter)?;
            qb.push(" LIMIT 1)");
        }
        AggregateKind::Sum { field } => {
            qb.push("(SELECT COALESCE(SUM(");
            qb.push(ident(dest_table)?);
            qb.push(".");
            qb.push(ident(field)?);
            qb.push("), 0) FROM ");
            qb.push(ident(dest_table)?);
            qb.push(" WHERE ");
            qb.push(ident(dest_table)?);
            qb.push(".");
            qb.push(ident(dest_attr)?);
            qb.push(" = ");
            qb.push(ident(source_table)?);
            qb.push(".");
            qb.push(ident(source_attr)?);
            push_aggregate_filter(qb, dest, &agg.filter)?;
            qb.push(")");
        }
    }
    Ok(())
}

fn push_aggregate_filter(
    qb: &mut QueryBuilder<'_, Sqlite>,
    dest: &ResourceDef,
    filter: &Option<AggregateFilter>,
) -> Result<()> {
    if let Some(f) = filter {
        qb.push(" AND ");
        match f {
            AggregateFilter::Eq(field, val) => {
                qb.push(ident(dest.table_name())?);
                qb.push(".");
                qb.push(ident(field)?);
                qb.push(" = ");
                push_sql_value(qb, &Value::from(*val));
            }
            AggregateFilter::Ne(field, val) => {
                qb.push(ident(dest.table_name())?);
                qb.push(".");
                qb.push(ident(field)?);
                qb.push(" <> ");
                push_sql_value(qb, &Value::from(*val));
            }
        }
    }
    Ok(())
}

fn push_expr(qb: &mut QueryBuilder<'_, Sqlite>, resource: &ResourceDef, expr: &Expr) -> Result<()> {
    match expr {
        Expr::Field(name) => {
            qb.push(column(resource, name)?);
        }
        Expr::LitInt(n) => {
            qb.push(n.to_string());
        }
        Expr::LitString(s) => {
            qb.push_bind((*s).to_string());
        }
        Expr::LitBool(b) => {
            qb.push(if *b { "1" } else { "0" });
        }
        Expr::Null => {
            qb.push("NULL");
        }
        Expr::StringLength(name) => {
            qb.push("length(");
            qb.push(column(resource, name)?);
            qb.push(")");
        }
        Expr::Length(inner) => {
            qb.push("length(");
            push_expr(qb, resource, inner)?;
            qb.push(")");
        }
        Expr::Lower(inner) => {
            qb.push("lower(");
            push_expr(qb, resource, inner)?;
            qb.push(")");
        }
        Expr::Upper(inner) => {
            qb.push("upper(");
            push_expr(qb, resource, inner)?;
            qb.push(")");
        }
        Expr::Concat(parts) => {
            qb.push("(");
            for (idx, part) in parts.iter().enumerate() {
                if idx > 0 {
                    qb.push(" || ");
                }
                push_expr(qb, resource, part)?;
            }
            qb.push(")");
        }
        Expr::Coalesce(parts) => {
            qb.push("COALESCE(");
            for (idx, part) in parts.iter().enumerate() {
                if idx > 0 {
                    qb.push(", ");
                }
                push_expr(qb, resource, part)?;
            }
            qb.push(")");
        }
        Expr::Add(l, r) => {
            qb.push("(");
            push_expr(qb, resource, l)?;
            qb.push(" + ");
            push_expr(qb, resource, r)?;
            qb.push(")");
        }
        Expr::Sub(l, r) => {
            qb.push("(");
            push_expr(qb, resource, l)?;
            qb.push(" - ");
            push_expr(qb, resource, r)?;
            qb.push(")");
        }
        Expr::Mul(l, r) => {
            qb.push("(");
            push_expr(qb, resource, l)?;
            qb.push(" * ");
            push_expr(qb, resource, r)?;
            qb.push(")");
        }
        Expr::Div(l, r) => {
            qb.push("(");
            push_expr(qb, resource, l)?;
            qb.push(" / ");
            push_expr(qb, resource, r)?;
            qb.push(")");
        }
        Expr::Eq(l, r) => {
            qb.push("(");
            push_expr(qb, resource, l)?;
            qb.push(" = ");
            push_expr(qb, resource, r)?;
            qb.push(")");
        }
        Expr::Ne(l, r) => {
            qb.push("(");
            push_expr(qb, resource, l)?;
            qb.push(" != ");
            push_expr(qb, resource, r)?;
            qb.push(")");
        }
        Expr::Gt(l, r) => {
            qb.push("(");
            push_expr(qb, resource, l)?;
            qb.push(" > ");
            push_expr(qb, resource, r)?;
            qb.push(")");
        }
        Expr::Gte(l, r) => {
            qb.push("(");
            push_expr(qb, resource, l)?;
            qb.push(" >= ");
            push_expr(qb, resource, r)?;
            qb.push(")");
        }
        Expr::Lt(l, r) => {
            qb.push("(");
            push_expr(qb, resource, l)?;
            qb.push(" < ");
            push_expr(qb, resource, r)?;
            qb.push(")");
        }
        Expr::Lte(l, r) => {
            qb.push("(");
            push_expr(qb, resource, l)?;
            qb.push(" <= ");
            push_expr(qb, resource, r)?;
            qb.push(")");
        }
        Expr::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            qb.push("(CASE WHEN ");
            push_expr(qb, resource, cond)?;
            qb.push(" THEN ");
            push_expr(qb, resource, then_expr)?;
            qb.push(" ELSE ");
            push_expr(qb, resource, else_expr)?;
            qb.push(" END)");
        }
        Expr::Custom(_) => {
            return Err(Error::Invalid(
                "custom code calculations cannot be directly compiled into SQL".into(),
            ));
        }
    }
    Ok(())
}

pub fn push_sql_value(qb: &mut QueryBuilder<'_, Sqlite>, value: &Value) {
    match value {
        Value::Null => {
            qb.push("NULL");
        }
        Value::Bool(flag) => {
            qb.push_bind(*flag);
        }
        Value::Int(int) => {
            qb.push_bind(*int);
        }
        Value::Uuid(id) => {
            qb.push_bind(id.to_string());
        }
        Value::String(text) => {
            qb.push_bind(text.clone());
        }
        Value::Map(map) => {
            let json = serde_json::to_string(map).unwrap_or_else(|_| "{}".to_string());
            qb.push_bind(json);
        }
        Value::Array(arr) => {
            let json = serde_json::to_string(arr).unwrap_or_else(|_| "[]".to_string());
            qb.push_bind(json);
        }
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
        embedded: false,
        data_layer: ash_core::DataLayerKind::Sqlite,
        timestamps: None,
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
}
