use ash_core::{
    AggregateDef, AggregateFilter, AggregateKind, AttrType, CompiledQuery, Error, Expr, FieldMap,
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

    pub fn sql(&self) -> &str {
        &self.sql
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
    pub current_calc_args: Option<FieldMap>,
}

impl<'a, D: SqlDialect> QueryCompiler<'a, D> {
    pub fn new(dialect: &'a D) -> Self {
        Self {
            dialect,
            param_counter: 0,
            params: Vec::new(),
            current_calc_args: None,
        }
    }

    /// Pushes a bound parameter and returns the dialect placeholder (e.g. `?` or `$1`).
    pub fn push_param(&mut self, val: Value) -> String {
        self.param_counter += 1;
        self.params.push(SqlParam::new(val));
        self.dialect.placeholder(self.param_counter)
    }

    fn bind_typed(&mut self, ty: AttrType, val: Value) -> String {
        let placeholder = self.push_param(val);
        self.dialect.cast_param(ty, &placeholder)
    }

    fn bind_field(&mut self, resource: &ResourceDef, field: &str, val: Value) -> String {
        match resource.attribute(field).map(|attr| attr.ty) {
            Some(ty) => self.bind_typed(ty, val),
            None => self.push_param(val),
        }
    }

    /// Pushes a bound list parameter and returns the dialect placeholder.
    pub fn push_list_param(&mut self, vals: Vec<Value>) -> String {
        self.param_counter += 1;
        self.params.push(SqlParam::list(vals));
        self.dialect.placeholder(self.param_counter)
    }

    pub fn compile_operand(&mut self, resource: &ResourceDef, field: &str) -> Result<String> {
        self.compile_operand_scoped(resource, field, None)
    }

    pub fn compile_operand_scoped(
        &mut self,
        resource: &ResourceDef,
        field: &str,
        scope_alias: Option<&str>,
    ) -> Result<String> {
        if let Some(calc) = resource.calculation(field) {
            self.compile_expr(resource, &calc.expr)
        } else {
            let col = column(self.dialect, resource, field)?;
            if let Some(alias) = scope_alias {
                Ok(format!("{alias}.{col}"))
            } else {
                Ok(col)
            }
        }
    }

    pub fn compile_expr(&mut self, resource: &ResourceDef, expr: &Expr) -> Result<String> {
        match expr {
            Expr::Field(name) => column(self.dialect, resource, name),
            Expr::Arg(name) => {
                let val = self
                    .current_calc_args
                    .as_ref()
                    .and_then(|args| args.get(*name))
                    .cloned()
                    .unwrap_or(Value::Null);
                let p = self.push_param(val);
                Ok(p)
            }
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
        self.compile_filter_scoped(resource, filter, None)
    }

    pub fn compile_filter_scoped(
        &mut self,
        resource: &ResourceDef,
        filter: &Filter,
        scope_alias: Option<&str>,
    ) -> Result<String> {
        match filter {
            Filter::True => Ok("1=1".to_string()),
            Filter::False => Ok("0=1".to_string()),
            Filter::Eq(field, val) if val.is_null() => {
                let op = self.compile_operand_scoped(resource, field, scope_alias)?;
                Ok(format!("{op} IS NULL"))
            }
            Filter::Eq(field, val) => {
                let op = self.compile_operand_scoped(resource, field, scope_alias)?;
                let p = self.bind_field(resource, field, val.clone());
                Ok(format!("{op} = {p}"))
            }
            Filter::Ne(field, val) if val.is_null() => {
                let op = self.compile_operand_scoped(resource, field, scope_alias)?;
                Ok(format!("{op} IS NOT NULL"))
            }
            Filter::Ne(field, val) => {
                let op = self.compile_operand_scoped(resource, field, scope_alias)?;
                let p = self.bind_field(resource, field, val.clone());
                Ok(format!("{op} <> {p}"))
            }
            Filter::Gt(field, val) => {
                let op = self.compile_operand_scoped(resource, field, scope_alias)?;
                let p = self.bind_field(resource, field, val.clone());
                Ok(format!("{op} > {p}"))
            }
            Filter::Gte(field, val) => {
                let op = self.compile_operand_scoped(resource, field, scope_alias)?;
                let p = self.bind_field(resource, field, val.clone());
                Ok(format!("{op} >= {p}"))
            }
            Filter::Lt(field, val) => {
                let op = self.compile_operand_scoped(resource, field, scope_alias)?;
                let p = self.bind_field(resource, field, val.clone());
                Ok(format!("{op} < {p}"))
            }
            Filter::Lte(field, val) => {
                let op = self.compile_operand_scoped(resource, field, scope_alias)?;
                let p = self.bind_field(resource, field, val.clone());
                Ok(format!("{op} <= {p}"))
            }
            Filter::IsNil(field) => {
                let op = self.compile_operand_scoped(resource, field, scope_alias)?;
                Ok(format!("{op} IS NULL"))
            }
            Filter::In(_field, vals) if vals.is_empty() => Ok("0=1".to_string()),
            Filter::In(field, vals) => {
                let op = self.compile_operand_scoped(resource, field, scope_alias)?;
                let param = self.push_list_param(vals.clone());
                Ok(self.dialect.render_in_list(&op, &param))
            }
            Filter::And(parts) => {
                let mut compiled = Vec::new();
                for part in parts {
                    compiled.push(self.compile_filter_scoped(resource, part, scope_alias)?);
                }
                Ok(format!("({})", compiled.join(" AND ")))
            }
            Filter::Or(parts) => {
                let mut compiled = Vec::new();
                for part in parts {
                    compiled.push(self.compile_filter_scoped(resource, part, scope_alias)?);
                }
                Ok(format!("({})", compiled.join(" OR ")))
            }
            Filter::Not(part) => {
                let inner = self.compile_filter_scoped(resource, part, scope_alias)?;
                Ok(format!("NOT ({inner})"))
            }
            Filter::Related {
                relationship,
                filter,
            } => {
                let rel = resource.relationship(relationship).ok_or_else(|| {
                    Error::Invalid(format!(
                        "unknown relationship `{relationship}` on {}",
                        resource.name
                    ))
                })?;
                let dest_res = (rel.destination)();
                self.param_counter += 1;
                let dest_alias = format!("rel_{}_{}", dest_res.table_name(), self.param_counter);
                let dest_table = ident(self.dialect, dest_res.table_name())?;
                let outer_scope = scope_alias
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| ident(self.dialect, resource.table_name()).unwrap());
                let outer_col = column(self.dialect, resource, rel.source_attribute)?;
                let dest_col = column(self.dialect, dest_res, rel.destination_attribute)?;
                let inner_sql = self.compile_filter_scoped(dest_res, filter, Some(&dest_alias))?;

                match rel.kind {
                    RelKind::ManyToMany => {
                        let through_fn = rel.through.ok_or_else(|| {
                            Error::Invalid(format!(
                                "many_to_many relationship `{}` on `{}` requires through join resource",
                                rel.name, resource.name
                            ))
                        })?;
                        let through_res = through_fn();
                        self.param_counter += 1;
                        let join_alias = format!(
                            "rel_join_{}_{}",
                            through_res.table_name(),
                            self.param_counter
                        );
                        let through_table = ident(self.dialect, through_res.table_name())?;
                        let source_on_join = column(
                            self.dialect,
                            through_res,
                            rel.source_attribute_on_join_resource
                                .unwrap_or(rel.source_attribute),
                        )?;
                        let dest_on_join = column(
                            self.dialect,
                            through_res,
                            rel.destination_attribute_on_join_resource
                                .unwrap_or(rel.destination_attribute),
                        )?;
                        Ok(format!(
                            "EXISTS (SELECT 1 FROM {dest_table} AS {dest_alias} INNER JOIN {through_table} AS {join_alias} ON {dest_alias}.{dest_col} = {join_alias}.{dest_on_join} WHERE {join_alias}.{source_on_join} = {outer_scope}.{outer_col} AND {inner_sql})"
                        ))
                    }
                    RelKind::BelongsTo | RelKind::HasMany | RelKind::HasOne => Ok(format!(
                        "EXISTS (SELECT 1 FROM {dest_table} AS {dest_alias} WHERE {dest_alias}.{dest_col} = {outer_scope}.{outer_col} AND {inner_sql})"
                    )),
                }
            }
        }
    }

    pub fn compile_aggregate_filter(
        &mut self,
        target_alias: &str,
        filter: &Option<AggregateFilter>,
    ) -> Result<String> {
        match filter {
            None => Ok(String::new()),
            Some(AggregateFilter::Eq(field, val)) => {
                let col = ident(self.dialect, field)?;
                let p = self.push_param(Value::from(*val));
                Ok(format!(" AND {target_alias}.{col} = {p}"))
            }
            Some(AggregateFilter::Ne(field, val)) => {
                let col = ident(self.dialect, field)?;
                let p = self.push_param(Value::from(*val));
                Ok(format!(" AND {target_alias}.{col} <> {p}"))
            }
        }
    }

    pub fn compile_aggregate(
        &mut self,
        resource: &ResourceDef,
        agg: &AggregateDef,
    ) -> Result<String> {
        let rel = resource.relationship(agg.relationship).ok_or_else(|| {
            Error::Invalid(format!("unknown relationship `{}`", agg.relationship))
        })?;
        let dest = (rel.destination)();
        let dest_table = ident(self.dialect, dest.table_name())?;
        let dest_alias = ident(self.dialect, &format!("_ash_sub_{}", agg.name))?;
        let source_table = ident(self.dialect, resource.table_name())?;
        let source_attr = ident(self.dialect, rel.source_attribute)?;
        let dest_attr = ident(self.dialect, rel.destination_attribute)?;

        match rel.kind {
            RelKind::HasMany | RelKind::BelongsTo | RelKind::HasOne => match &agg.kind {
                AggregateKind::Count => {
                    let mut s = format!(
                        "(SELECT COUNT(*) FROM {dest_table} AS {dest_alias} WHERE {dest_alias}.{dest_attr} = {source_table}.{source_attr}"
                    );
                    s.push_str(&self.compile_aggregate_filter(&dest_alias, &agg.filter)?);
                    s.push(')');
                    Ok(s)
                }
                AggregateKind::Exists => {
                    let mut s = format!(
                        "(EXISTS (SELECT 1 FROM {dest_table} AS {dest_alias} WHERE {dest_alias}.{dest_attr} = {source_table}.{source_attr}"
                    );
                    s.push_str(&self.compile_aggregate_filter(&dest_alias, &agg.filter)?);
                    s.push_str("))");
                    Ok(s)
                }
                AggregateKind::First { field } => {
                    let f = ident(self.dialect, field)?;
                    let mut s = format!(
                        "(SELECT {dest_alias}.{f} FROM {dest_table} AS {dest_alias} WHERE {dest_alias}.{dest_attr} = {source_table}.{source_attr}"
                    );
                    s.push_str(&self.compile_aggregate_filter(&dest_alias, &agg.filter)?);
                    s.push_str(" LIMIT 1)");
                    Ok(s)
                }
                AggregateKind::Sum { field } => {
                    let f = ident(self.dialect, field)?;
                    let mut s = format!(
                        "(SELECT SUM({dest_alias}.{f}) FROM {dest_table} AS {dest_alias} WHERE {dest_alias}.{dest_attr} = {source_table}.{source_attr}"
                    );
                    s.push_str(&self.compile_aggregate_filter(&dest_alias, &agg.filter)?);
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
                let join_alias = ident(self.dialect, &format!("_ash_join_{}", agg.name))?;
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
                            "(SELECT COUNT(*) FROM {dest_table} AS {dest_alias} JOIN {join_table} AS {join_alias} ON {dest_alias}.{dest_attr} = {join_alias}.{dest_on_join} WHERE {join_alias}.{source_on_join} = {source_table}.{source_attr}"
                        );
                        s.push_str(&self.compile_aggregate_filter(&dest_alias, &agg.filter)?);
                        s.push(')');
                        Ok(s)
                    }
                    AggregateKind::Exists => {
                        let mut s = format!(
                            "(EXISTS (SELECT 1 FROM {dest_table} AS {dest_alias} JOIN {join_table} AS {join_alias} ON {dest_alias}.{dest_attr} = {join_alias}.{dest_on_join} WHERE {join_alias}.{source_on_join} = {source_table}.{source_attr}"
                        );
                        s.push_str(&self.compile_aggregate_filter(&dest_alias, &agg.filter)?);
                        s.push_str("))");
                        Ok(s)
                    }
                    AggregateKind::First { field } => {
                        let f = ident(self.dialect, field)?;
                        let mut s = format!(
                            "(SELECT {dest_alias}.{f} FROM {dest_table} AS {dest_alias} JOIN {join_table} AS {join_alias} ON {dest_alias}.{dest_attr} = {join_alias}.{dest_on_join} WHERE {join_alias}.{source_on_join} = {source_table}.{source_attr}"
                        );
                        s.push_str(&self.compile_aggregate_filter(&dest_alias, &agg.filter)?);
                        s.push_str(" LIMIT 1)");
                        Ok(s)
                    }
                    AggregateKind::Sum { field } => {
                        let f = ident(self.dialect, field)?;
                        let mut s = format!(
                            "(SELECT SUM({dest_alias}.{f}) FROM {dest_table} AS {dest_alias} JOIN {join_table} AS {join_alias} ON {dest_alias}.{dest_attr} = {join_alias}.{dest_on_join} WHERE {join_alias}.{source_on_join} = {source_table}.{source_attr}"
                        );
                        s.push_str(&self.compile_aggregate_filter(&dest_alias, &agg.filter)?);
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
            let pk = resource.primary_key().map(|p| p.name).unwrap_or("id");
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
                    let p = self.bind_field(resource, &prev.field, val.clone());
                    prefix_match.push(format!("{prev_col} = {p}"));
                }
            }

            let col = column(self.dialect, resource, &sort.field)?;
            let op = if sort.descending { "<" } else { ">" };
            if let Some((_, val)) = cursor.values.iter().find(|(k, _)| k == &sort.field) {
                let p = self.bind_field(resource, &sort.field, val.clone());
                let mut branch = format!("{col} {op} {p}");
                if !prefix_match.is_empty() {
                    branch = format!("{} AND {branch}", prefix_match.join(" AND "));
                }
                conds.push(format!("({branch})"));
            }
        }

        // Add deterministic tie-breaker on primary key if not explicitly in sort fields
        let pk = resource.primary_key().map(|p| p.name).unwrap_or("id");
        if !sorts.iter().any(|s| s.field == pk) {
            let mut prefix_match = Vec::new();
            for prev in sorts {
                let prev_col = column(self.dialect, resource, &prev.field)?;
                if let Some((_, val)) = cursor.values.iter().find(|(k, _)| k == &prev.field) {
                    let p = self.bind_field(resource, &prev.field, val.clone());
                    prefix_match.push(format!("{prev_col} = {p}"));
                }
            }
            let pk_col = column(self.dialect, resource, pk)?;
            let p = self.push_param(Value::Uuid(cursor.id));
            let mut branch = format!("{pk_col} > {p}");
            if !prefix_match.is_empty() {
                branch = format!("{} AND {branch}", prefix_match.join(" AND "));
            }
            conds.push(format!("({branch})"));
        }

        if conds.is_empty() {
            let pk = resource.primary_key().map(|p| p.name).unwrap_or("id");
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
                Error::Invalid(format!(
                    "unknown calculation `{calc_name}` on {}",
                    resource.name
                ))
            })?;
            self.current_calc_args = query.calculation_args.get(calc_name).cloned();
            let expr_sql = self.compile_expr(resource, &calc.expr)?;
            self.current_calc_args = None;
            let alias = ident(self.dialect, calc.name)?;
            select_items.push(format!("{expr_sql} AS {alias}"));
        }

        for agg_name in &query.aggregates {
            let agg = resource.aggregate(agg_name).ok_or_else(|| {
                Error::Invalid(format!(
                    "unknown aggregate `{agg_name}` on {}",
                    resource.name
                ))
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

        let mut all_sorts = query.sort.clone();
        if cursor.is_some() {
            let pk = resource.primary_key().map(|p| p.name).unwrap_or("id");
            if !all_sorts.iter().any(|s| s.field == pk) {
                all_sorts.push(Sort {
                    field: pk.to_string(),
                    descending: false,
                });
            }
        }
        sql.push_str(&self.compile_sort(resource, &all_sorts)?);

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
                placeholders.push(self.bind_typed(attr.ty, val.clone()));
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

    pub fn compile_bulk_insert(
        &mut self,
        resource: &ResourceDef,
        rows: &[(Uuid, FieldMap)],
    ) -> Result<CompiledSql> {
        if rows.is_empty() {
            return Ok(CompiledSql::new(String::new(), Vec::new()));
        }
        let table = ident(self.dialect, resource.table_name())?;

        let mut col_names = Vec::new();
        let mut attrs = Vec::new();
        for attr in resource.attributes {
            col_names.push(ident(self.dialect, attr.name)?);
            attrs.push(attr);
        }

        let mut row_placeholders = Vec::with_capacity(rows.len());
        for (id, fields) in rows {
            let mut placeholders = Vec::with_capacity(attrs.len());
            for attr in &attrs {
                let val = if attr.primary_key {
                    fields.get(attr.name).cloned().unwrap_or(Value::Uuid(*id))
                } else {
                    fields.get(attr.name).cloned().unwrap_or(Value::Null)
                };
                placeholders.push(self.bind_typed(attr.ty, val));
            }
            row_placeholders.push(format!("({})", placeholders.join(", ")));
        }

        let mut sql = format!(
            "INSERT INTO {table} ({}) VALUES {}",
            col_names.join(", "),
            row_placeholders.join(", ")
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
                let p = self.bind_typed(attr.ty, val.clone());
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
            return Ok(CompiledSql::new(
                format!("DELETE FROM {table} WHERE 0=1"),
                Vec::new(),
            ));
        }

        let vals: Vec<Value> = ids.iter().map(|id| Value::Uuid(*id)).collect();
        let param = self.push_list_param(vals);
        let condition = self.dialect.render_in_list(&pk_col, &param);

        let sql = format!("DELETE FROM {table} WHERE {condition}");
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
            compiled
                .sql
                .truncate(compiled.sql.len() - " RETURNING *".len());
        }

        compiled.sql.push(' ');
        compiled
            .sql
            .push_str(&self.dialect.upsert_clause(identity, update_fields));

        if self.dialect.supports_returning() {
            compiled.sql.push_str(" RETURNING *");
        }

        Ok(compiled)
    }

    pub fn compile_create_table(&self, resource: &ResourceDef) -> Result<String> {
        Ok(crate::generator::emit_create_table(
            self.dialect,
            &crate::snapshot::TableSnapshot::from_resource(resource, self.dialect),
        ))
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
