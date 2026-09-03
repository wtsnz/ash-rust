use ash_core::{
    AggregateDef, AggregateFilter, AggregateKind, CompiledQuery, Error, Expr, FieldMap,
    Filter, IdentityDef, KeysetCursor, RelKind, ResourceDef, Result, Sort, Value,
};
use uuid::Uuid;

use crate::dialect::SqlDialect;
use crate::param::SqlParam;

/// A parameterized SQL statement and its bound parameter values.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledSql {
    pub sql: String,
    pub params: Vec<SqlParam>,
}

impl CompiledSql {
    pub fn new(sql: String, params: Vec<SqlParam>) -> Self {
        Self { sql, params }
    }
}

/// Validates that an identifier contains only ASCII alphanumeric characters or underscores,
/// and returns it quoted according to the dialect.
pub fn ident<D: SqlDialect>(dialect: &D, name: &str) -> Result<String> {
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(Error::Invalid(format!("invalid identifier `{name}`")));
    }
    Ok(dialect.quote_identifier(name))
}

/// Validates that a column exists on the resource and returns its quoted identifier.
pub fn column<D: SqlDialect>(dialect: &D, resource: &ResourceDef, field: &str) -> Result<String> {
    if resource.attribute(field).is_none()
        && resource.calculation(field).is_none()
        && resource.aggregate(field).is_none()
    {
        return Err(Error::Invalid(format!(
            "unknown field `{field}` on {}",
            resource.name
        )));
    }
    ident(dialect, field)
}

/// Query and expression compiler transforming Ash DSL queries into dialect-specific SQL.
pub struct QueryCompiler<'a, D: SqlDialect> {
    pub dialect: &'a D,
    param_counter: usize,
    pub params: Vec<SqlParam>,
}

impl<'a, D: SqlDialect> QueryCompiler<'a, D> {
    pub fn new(dialect: &'a D) -> Self {
        Self {
            dialect,
            param_counter: 0,
            params: Vec::new(),
        }
    }

    /// Pushes a bound parameter and returns the dialect placeholder (e.g. `?` or `$1`).
    pub fn push_param(&mut self, val: Value) -> String {
        self.param_counter += 1;
        self.params.push(SqlParam::new(val));
        self.dialect.placeholder(self.param_counter)
    }

    pub fn compile_operand(&mut self, resource: &ResourceDef, field: &str) -> Result<String> {
        if let Some(calc) = resource.calculation(field) {
            self.compile_expr(resource, &calc.expr)
        } else {
            column(self.dialect, resource, field)
        }
    }

    pub fn compile_expr(&mut self, resource: &ResourceDef, expr: &Expr) -> Result<String> {
        match expr {
            Expr::Field(name) => column(self.dialect, resource, name),
            Expr::LitInt(n) => Ok(n.to_string()),
            Expr::LitString(s) => {
                let p = self.push_param(Value::String((*s).to_string()));
                Ok(p)
            }
            Expr::LitBool(b) => Ok(self.dialect.boolean_literal(*b).to_string()),
            Expr::Null => Ok("NULL".to_string()),
            Expr::StringLength(name) => {
                let col = column(self.dialect, resource, name)?;
                Ok(format!("length({col})"))
            }
            Expr::Length(inner) => {
                let e = self.compile_expr(resource, inner)?;
                Ok(format!("length({e})"))
            }
            Expr::Lower(inner) => {
                let e = self.compile_expr(resource, inner)?;
                Ok(format!("lower({e})"))
            }
            Expr::Upper(inner) => {
                let e = self.compile_expr(resource, inner)?;
                Ok(format!("upper({e})"))
            }
            Expr::Concat(parts) => {
                let mut compiled = Vec::new();
                for part in (*parts).iter() {
                    compiled.push(self.compile_expr(resource, part)?);
                }
                Ok(format!("({})", compiled.join(" || ")))
            }
            Expr::Coalesce(parts) => {
                let mut compiled = Vec::new();
                for part in (*parts).iter() {
                    compiled.push(self.compile_expr(resource, part)?);
                }
                Ok(format!("COALESCE({})", compiled.join(", ")))
            }
            Expr::Add(l, r) => {
                let left = self.compile_expr(resource, l)?;
                let right = self.compile_expr(resource, r)?;
                Ok(format!("({left} + {right})"))
            }
            Expr::Sub(l, r) => {
                let left = self.compile_expr(resource, l)?;
                let right = self.compile_expr(resource, r)?;
                Ok(format!("({left} - {right})"))
            }
            Expr::Mul(l, r) => {
                let left = self.compile_expr(resource, l)?;
                let right = self.compile_expr(resource, r)?;
                Ok(format!("({left} * {right})"))
            }
            Expr::Div(l, r) => {
                let left = self.compile_expr(resource, l)?;
                let right = self.compile_expr(resource, r)?;
                Ok(format!("({left} / {right})"))
            }
            Expr::Eq(l, r) => {
                let left = self.compile_expr(resource, l)?;
                let right = self.compile_expr(resource, r)?;
                Ok(format!("({left} = {right})"))
            }
            Expr::Ne(l, r) => {
                let left = self.compile_expr(resource, l)?;
                let right = self.compile_expr(resource, r)?;
                Ok(format!("({left} <> {right})"))
            }
            Expr::Gt(l, r) => {
                let left = self.compile_expr(resource, l)?;
                let right = self.compile_expr(resource, r)?;
                Ok(format!("({left} > {right})"))
            }
            Expr::Gte(l, r) => {
                let left = self.compile_expr(resource, l)?;
                let right = self.compile_expr(resource, r)?;
                Ok(format!("({left} >= {right})"))
            }
            Expr::Lt(l, r) => {
                let left = self.compile_expr(resource, l)?;
                let right = self.compile_expr(resource, r)?;
                Ok(format!("({left} < {right})"))
            }
            Expr::Lte(l, r) => {
                let left = self.compile_expr(resource, l)?;
                let right = self.compile_expr(resource, r)?;
                Ok(format!("({left} <= {right})"))
            }
            Expr::IfElse {
                cond,
                then_expr,
                else_expr,
            } => {
                let c = self.compile_expr(resource, cond)?;
                let t = self.compile_expr(resource, then_expr)?;
                let e = self.compile_expr(resource, else_expr)?;
                Ok(format!("(CASE WHEN {c} THEN {t} ELSE {e} END)"))
            }
            Expr::Custom(_) => Err(Error::Invalid(
                "custom code calculations cannot be directly compiled into SQL".into(),
            )),
        }
    }

    pub fn compile_filter(&mut self, resource: &ResourceDef, filter: &Filter) -> Result<String> {
        match filter {
            Filter::True => Ok("1=1".to_string()),
            Filter::False => Ok("0=1".to_string()),
            Filter::Eq(field, val) if val.is_null() => {
                let op = self.compile_operand(resource, field)?;
                Ok(format!("{op} IS NULL"))
            }
            Filter::Eq(field, val) => {
                let op = self.compile_operand(resource, field)?;
                let p = self.push_param(val.clone());
                Ok(format!("{op} = {p}"))
            }
            Filter::Ne(field, val) if val.is_null() => {
                let op = self.compile_operand(resource, field)?;
                Ok(format!("{op} IS NOT NULL"))
            }
            Filter::Ne(field, val) => {
                let op = self.compile_operand(resource, field)?;
                let p = self.push_param(val.clone());
                Ok(format!("{op} <> {p}"))
            }
            Filter::Gt(field, val) => {
                let op = self.compile_operand(resource, field)?;
                let p = self.push_param(val.clone());
                Ok(format!("{op} > {p}"))
            }
            Filter::Gte(field, val) => {
                let op = self.compile_operand(resource, field)?;
                let p = self.push_param(val.clone());
                Ok(format!("{op} >= {p}"))
            }
            Filter::Lt(field, val) => {
                let op = self.compile_operand(resource, field)?;
                let p = self.push_param(val.clone());
                Ok(format!("{op} < {p}"))
            }
            Filter::Lte(field, val) => {
                let op = self.compile_operand(resource, field)?;
                let p = self.push_param(val.clone());
                Ok(format!("{op} <= {p}"))
            }
            Filter::IsNil(field) => {
                let op = self.compile_operand(resource, field)?;
                Ok(format!("{op} IS NULL"))
            }
            Filter::In(_field, vals) if vals.is_empty() => Ok("0=1".to_string()),
            Filter::In(field, vals) => {
                let op = self.compile_operand(resource, field)?;
                let mut placeholders = Vec::new();
                for val in vals {
                    placeholders.push(self.push_param(val.clone()));
                }
                Ok(format!("{op} IN ({})", placeholders.join(", ")))
            }
            Filter::And(parts) => {
                let mut compiled = Vec::new();
                for part in parts {
                    compiled.push(self.compile_filter(resource, part)?);
                }
                Ok(format!("({})", compiled.join(" AND ")))
            }
            Filter::Or(parts) => {
                let mut compiled = Vec::new();
                for part in parts {
                    compiled.push(self.compile_filter(resource, part)?);
                }
                Ok(format!("({})", compiled.join(" OR ")))
            }
            Filter::Not(part) => {
                let inner = self.compile_filter(resource, part)?;
                Ok(format!("NOT ({inner})"))
            }
        }
    }

    pub fn compile_aggregate_filter(
        &mut self,
        dest: &ResourceDef,
        filter: &Option<AggregateFilter>,
    ) -> Result<String> {
        match filter {
            None => Ok(String::new()),
            Some(AggregateFilter::Eq(field, val)) => {
                let table = ident(self.dialect, dest.table_name())?;
                let col = ident(self.dialect, field)?;
                let p = self.push_param(Value::from(*val));
                Ok(format!(" AND {table}.{col} = {p}"))
            }
            Some(AggregateFilter::Ne(field, val)) => {
                let table = ident(self.dialect, dest.table_name())?;
                let col = ident(self.dialect, field)?;
                let p = self.push_param(Value::from(*val));
                Ok(format!(" AND {table}.{col} <> {p}"))
            }
        }
    }

    pub fn compile_aggregate(
        &mut self,
        resource: &ResourceDef,
        agg: &AggregateDef,
    ) -> Result<String> {
        let rel = resource
            .relationship(agg.relationship)
            .ok_or_else(|| Error::Invalid(format!("unknown relationship `{}`", agg.relationship)))?;
        let dest = (rel.destination)();
        let dest_table = ident(self.dialect, dest.table_name())?;
        let source_table = ident(self.dialect, resource.table_name())?;
        let source_attr = ident(self.dialect, rel.source_attribute)?;
        let dest_attr = ident(self.dialect, rel.destination_attribute)?;

        match rel.kind {
            RelKind::HasMany | RelKind::BelongsTo => match &agg.kind {
                AggregateKind::Count => {
                    let mut s = format!(
                        "(SELECT COUNT(*) FROM {dest_table} WHERE {dest_table}.{dest_attr} = {source_table}.{source_attr}"
                    );
                    s.push_str(&self.compile_aggregate_filter(dest, &agg.filter)?);
                    s.push(')');
                    Ok(s)
                }
                AggregateKind::Exists => {
                    let mut s = format!(
                        "(EXISTS (SELECT 1 FROM {dest_table} WHERE {dest_table}.{dest_attr} = {source_table}.{source_attr}"
                    );
                    s.push_str(&self.compile_aggregate_filter(dest, &agg.filter)?);
                    s.push_str("))");
                    Ok(s)
                }
                AggregateKind::First { field } => {
                    let f = ident(self.dialect, field)?;
                    let mut s = format!(
                        "(SELECT {dest_table}.{f} FROM {dest_table} WHERE {dest_table}.{dest_attr} = {source_table}.{source_attr}"
                    );
                    s.push_str(&self.compile_aggregate_filter(dest, &agg.filter)?);
                    s.push_str(" LIMIT 1)");
                    Ok(s)
                }
                AggregateKind::Sum { field } => {
                    let f = ident(self.dialect, field)?;
                    let mut s = format!(
                        "(SELECT SUM({dest_table}.{f}) FROM {dest_table} WHERE {dest_table}.{dest_attr} = {source_table}.{source_attr}"
                    );
                    s.push_str(&self.compile_aggregate_filter(dest, &agg.filter)?);
                    s.push(')');
                    Ok(s)
                }
            },
            RelKind::ManyToMany => {
                let through_fn = rel.through.ok_or_else(|| {
                    Error::Invalid(format!(
                        "many_to_many relationship `{}` must specify a join resource",
                        rel.name
                    ))
                })?;
                let through_def = through_fn();
                let join_table = ident(self.dialect, through_def.table_name())?;
                let source_on_join = ident(
                    self.dialect,
                    rel.source_attribute_on_join_resource
                        .unwrap_or(rel.source_attribute),
                )?;
                let dest_on_join = ident(
                    self.dialect,
                    rel.destination_attribute_on_join_resource
                        .unwrap_or(rel.destination_attribute),
                )?;

                match &agg.kind {
                    AggregateKind::Count => {
                        let mut s = format!(
                            "(SELECT COUNT(*) FROM {dest_table} JOIN {join_table} ON {dest_table}.{dest_attr} = {join_table}.{dest_on_join} WHERE {join_table}.{source_on_join} = {source_table}.{source_attr}"
                        );
                        s.push_str(&self.compile_aggregate_filter(dest, &agg.filter)?);
                        s.push(')');
                        Ok(s)
                    }
                    AggregateKind::Exists => {
                        let mut s = format!(
                            "(EXISTS (SELECT 1 FROM {dest_table} JOIN {join_table} ON {dest_table}.{dest_attr} = {join_table}.{dest_on_join} WHERE {join_table}.{source_on_join} = {source_table}.{source_attr}"
                        );
                        s.push_str(&self.compile_aggregate_filter(dest, &agg.filter)?);
                        s.push_str("))");
                        Ok(s)
                    }
                    AggregateKind::First { field } => {
                        let f = ident(self.dialect, field)?;
                        let mut s = format!(
                            "(SELECT {dest_table}.{f} FROM {dest_table} JOIN {join_table} ON {dest_table}.{dest_attr} = {join_table}.{dest_on_join} WHERE {join_table}.{source_on_join} = {source_table}.{source_attr}"
                        );
                        s.push_str(&self.compile_aggregate_filter(dest, &agg.filter)?);
                        s.push_str(" LIMIT 1)");
                        Ok(s)
                    }
                    AggregateKind::Sum { field } => {
                        let f = ident(self.dialect, field)?;
                        let mut s = format!(
                            "(SELECT SUM({dest_table}.{f}) FROM {dest_table} JOIN {join_table} ON {dest_table}.{dest_attr} = {join_table}.{dest_on_join} WHERE {join_table}.{source_on_join} = {source_table}.{source_attr}"
                        );
                        s.push_str(&self.compile_aggregate_filter(dest, &agg.filter)?);
                        s.push(')');
                        Ok(s)
                    }
                }
            }
        }
    }

    pub fn compile_sort(&self, resource: &ResourceDef, sorts: &[Sort]) -> Result<String> {
        if sorts.is_empty() {
            return Ok(String::new());
        }
        let mut clauses = Vec::new();
        for sort in sorts {
            let col = column(self.dialect, resource, &sort.field)?;
            let dir = if sort.descending { "DESC" } else { "ASC" };
            clauses.push(format!("{col} {dir}"));
        }
        Ok(format!(" ORDER BY {}", clauses.join(", ")))
    }

    pub fn compile_keyset_cursor(
        &mut self,
        resource: &ResourceDef,
        cursor: &KeysetCursor,
        sorts: &[Sort],
    ) -> Result<String> {
        if sorts.is_empty() {
            let pk = resource
                .primary_key()
                .map(|p| p.name)
                .unwrap_or("id");
            let pk_col = column(self.dialect, resource, pk)?;
            let p = self.push_param(Value::Uuid(cursor.id));
            return Ok(format!("{pk_col} > {p}"));
        }

        let mut conds = Vec::new();
        for (i, sort) in sorts.iter().enumerate() {
            let mut prefix_match = Vec::new();
            for prev in sorts.iter().take(i) {
                let prev_col = column(self.dialect, resource, &prev.field)?;
                if let Some((_, val)) = cursor.values.iter().find(|(k, _)| k == &prev.field) {
                    let p = self.push_param(val.clone());
                    prefix_match.push(format!("{prev_col} = {p}"));
                }
            }

            let col = column(self.dialect, resource, &sort.field)?;
            let op = if sort.descending { "<" } else { ">" };
            if let Some((_, val)) = cursor.values.iter().find(|(k, _)| k == &sort.field) {
                let p = self.push_param(val.clone());
                let mut branch = format!("{col} {op} {p}");
                if !prefix_match.is_empty() {
                    branch = format!("{} AND {branch}", prefix_match.join(" AND "));
                }
                conds.push(format!("({branch})"));
            }
        }

        if conds.is_empty() {
            let pk = resource
                .primary_key()
                .map(|p| p.name)
                .unwrap_or("id");
            let pk_col = column(self.dialect, resource, pk)?;
            let p = self.push_param(Value::Uuid(cursor.id));
            Ok(format!("{pk_col} > {p}"))
        } else {
            Ok(format!("({})", conds.join(" OR ")))
        }
    }

    pub fn compile_select(
        &mut self,
        resource: &ResourceDef,
        query: &CompiledQuery,
    ) -> Result<CompiledSql> {
        self.compile_select_internal(resource, query, None)
    }

    pub fn compile_select_with_cursor(
        &mut self,
        resource: &ResourceDef,
        query: &CompiledQuery,
        cursor: Option<&KeysetCursor>,
    ) -> Result<CompiledSql> {
        self.compile_select_internal(resource, query, cursor)
    }

    fn compile_select_internal(
        &mut self,
        resource: &ResourceDef,
        query: &CompiledQuery,
        cursor: Option<&KeysetCursor>,
    ) -> Result<CompiledSql> {
        let mut sql = String::from("SELECT ");
        let mut select_items = Vec::new();

        for attr in resource.attributes {
            select_items.push(ident(self.dialect, attr.name)?);
        }

        for calc_name in &query.calculations {
            let calc = resource.calculation(calc_name).ok_or_else(|| {
                Error::Invalid(format!("unknown calculation `{calc_name}` on {}", resource.name))
            })?;
            let expr_sql = self.compile_expr(resource, &calc.expr)?;
            let alias = ident(self.dialect, calc.name)?;
            select_items.push(format!("{expr_sql} AS {alias}"));
        }

        for agg_name in &query.aggregates {
            let agg = resource.aggregate(agg_name).ok_or_else(|| {
                Error::Invalid(format!("unknown aggregate `{agg_name}` on {}", resource.name))
            })?;
            let agg_sql = self.compile_aggregate(resource, agg)?;
            let alias = ident(self.dialect, agg.name)?;
            select_items.push(format!("{agg_sql} AS {alias}"));
        }

        sql.push_str(&select_items.join(", "));
        sql.push_str(" FROM ");
        sql.push_str(&ident(self.dialect, resource.table_name())?);

        let mut where_clauses = Vec::new();

        if let Some(filter) = &query.filter {
            where_clauses.push(self.compile_filter(resource, filter)?);
        }

        if let Some(c) = cursor {
            where_clauses.push(self.compile_keyset_cursor(resource, c, &query.sort)?);
        }

        if !where_clauses.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&where_clauses.join(" AND "));
        }

        sql.push_str(&self.compile_sort(resource, &query.sort)?);

        if let Some(limit) = query.limit {
            sql.push_str(" LIMIT ");
            let p = self.push_param(Value::Int(limit as i64));
            sql.push_str(&p);
        }

        if let Some(offset) = query.offset {
            sql.push_str(" OFFSET ");
            let p = self.push_param(Value::Int(offset as i64));
            sql.push_str(&p);
        }

        Ok(CompiledSql::new(sql, self.params.clone()))
    }

    pub fn compile_insert(
        &mut self,
        resource: &ResourceDef,
        fields: &FieldMap,
    ) -> Result<CompiledSql> {
        let table = ident(self.dialect, resource.table_name())?;
        let mut col_names = Vec::new();
        let mut placeholders = Vec::new();

        for attr in resource.attributes {
            if let Some(val) = fields.get(attr.name) {
                col_names.push(ident(self.dialect, attr.name)?);
                placeholders.push(self.push_param(val.clone()));
            }
        }

        let mut sql = format!(
            "INSERT INTO {table} ({}) VALUES ({})",
            col_names.join(", "),
            placeholders.join(", ")
        );

        if self.dialect.supports_returning() {
            sql.push_str(" RETURNING *");
        }

        Ok(CompiledSql::new(sql, self.params.clone()))
    }

    pub fn compile_update(
        &mut self,
        resource: &ResourceDef,
        id: Uuid,
        fields: &FieldMap,
    ) -> Result<CompiledSql> {
        let pk = resource
            .primary_key()
            .ok_or(Error::NoPrimaryKey(resource.name))?;
        let table = ident(self.dialect, resource.table_name())?;

        let mut set_clauses = Vec::new();
        for attr in resource.attributes {
            if attr.primary_key {
                continue;
            }
            if let Some(val) = fields.get(attr.name) {
                let col = ident(self.dialect, attr.name)?;
                let p = self.push_param(val.clone());
                set_clauses.push(format!("{col} = {p}"));
            }
        }

        if set_clauses.is_empty() {
            let pk_col = ident(self.dialect, pk.name)?;
            set_clauses.push(format!("{pk_col} = {pk_col}"));
        }

        let pk_col = ident(self.dialect, pk.name)?;
        let pk_param = self.push_param(Value::Uuid(id));

        let mut sql = format!(
            "UPDATE {table} SET {} WHERE {pk_col} = {pk_param}",
            set_clauses.join(", ")
        );

        if let Some(v_attr) = resource.optimistic_lock_attribute()
            && let Some(Value::Int(new_v)) = fields.get(v_attr)
        {
            let expected_v = new_v - 1;
            let v_col = ident(self.dialect, v_attr)?;
            let v_param = self.push_param(Value::Int(expected_v));
            sql.push_str(&format!(" AND {v_col} = {v_param}"));
        }

        if self.dialect.supports_returning() {
            sql.push_str(" RETURNING *");
        }

        Ok(CompiledSql::new(sql, self.params.clone()))
    }

    pub fn compile_delete(&mut self, resource: &ResourceDef, id: Uuid) -> Result<CompiledSql> {
        let pk = resource
            .primary_key()
            .ok_or(Error::NoPrimaryKey(resource.name))?;
        let table = ident(self.dialect, resource.table_name())?;
        let pk_col = ident(self.dialect, pk.name)?;
        let p = self.push_param(Value::Uuid(id));

        let sql = format!("DELETE FROM {table} WHERE {pk_col} = {p}");
        Ok(CompiledSql::new(sql, self.params.clone()))
    }

    pub fn compile_bulk_delete(
        &mut self,
        resource: &ResourceDef,
        ids: &[Uuid],
    ) -> Result<CompiledSql> {
        let pk = resource
            .primary_key()
            .ok_or(Error::NoPrimaryKey(resource.name))?;
        let table = ident(self.dialect, resource.table_name())?;
        let pk_col = ident(self.dialect, pk.name)?;

        if ids.is_empty() {
            return Ok(CompiledSql::new(format!("DELETE FROM {table} WHERE 0=1"), Vec::new()));
        }

        let mut placeholders = Vec::new();
        for id in ids {
            placeholders.push(self.push_param(Value::Uuid(*id)));
        }

        let sql = format!("DELETE FROM {table} WHERE {pk_col} IN ({})", placeholders.join(", "));
        Ok(CompiledSql::new(sql, self.params.clone()))
    }

    pub fn compile_upsert(
        &mut self,
        resource: &ResourceDef,
        fields: &FieldMap,
        identity: &IdentityDef,
        update_fields: &[String],
    ) -> Result<CompiledSql> {
        let mut compiled = self.compile_insert(resource, fields)?;
        let has_returning = compiled.sql.ends_with(" RETURNING *");
        if has_returning {
            compiled.sql.truncate(compiled.sql.len() - " RETURNING *".len());
        }

        compiled.sql.push(' ');
        compiled.sql.push_str(&self.dialect.upsert_clause(identity, update_fields));

        if self.dialect.supports_returning() {
            compiled.sql.push_str(" RETURNING *");
        }

        Ok(compiled)
    }

    pub fn compile_create_table(&self, resource: &ResourceDef) -> Result<String> {
        let table = ident(self.dialect, resource.table_name())?;
        let mut cols = Vec::new();

        for attr in resource.attributes {
            let name = ident(self.dialect, attr.name)?;
            let ty = self.dialect.column_type(attr);
            let mut col = format!("{name} {ty}");
            if attr.primary_key {
                col.push_str(" PRIMARY KEY");
            }
            if !attr.allow_nil && !attr.primary_key {
                col.push_str(" NOT NULL");
            }
            cols.push(col);
        }

        for identity in resource.identities {
            let mut key_cols = Vec::new();
            for key in identity.keys {
                key_cols.push(ident(self.dialect, key)?);
            }
            cols.push(format!("UNIQUE ({})", key_cols.join(", ")));
        }

        let if_not_exists = if self.dialect.create_table_if_not_exists() {
            "IF NOT EXISTS "
        } else {
            ""
        };

        Ok(format!("CREATE TABLE {if_not_exists}{table} ({})", cols.join(", ")))
    }

    pub fn compile_create_indexes(&self, resource: &ResourceDef) -> Result<Vec<String>> {
        let mut stmts = Vec::new();
        let table = ident(self.dialect, resource.table_name())?;

        for identity in resource.identities {
            let idx_name = ident(
                self.dialect,
                &format!("idx_{}_{}", resource.table_name(), identity.name),
            )?;
            let mut key_cols = Vec::new();
            for key in identity.keys {
                key_cols.push(ident(self.dialect, key)?);
            }
            let if_not_exists = if self.dialect.create_table_if_not_exists() {
                "IF NOT EXISTS "
            } else {
                ""
            };
            stmts.push(format!(
                "CREATE UNIQUE INDEX {if_not_exists}{idx_name} ON {table} ({})",
                key_cols.join(", ")
            ));
        }

        Ok(stmts)
    }
}
