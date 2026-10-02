use ash_core::{
    AtomicCondition, AtomicExpr, AtomicUpdate,
    AggregateDef, AggregateFilter, AggregateKind, AttrType, CalculationDef, CompiledQuery, Error,
    Expr, FieldMap, Filter, IdentityDef, KeysetCursor, RelKind, ResourceDef, Result, Sort, Value,
};
use uuid::Uuid;

use crate::dialect::{SqlDialect, TextMatch};
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

/// `dest_alias.dest_col = outer_alias.source_col`, AND-ed when the relationship is composite.
fn relationship_equalities<D: SqlDialect>(
    dialect: &D,
    dest_alias: &str,
    dest: &ResourceDef,
    outer_alias: &str,
    outer: &ResourceDef,
    rel: &ash_core::RelationshipDef,
) -> Result<String> {
    let source_cols = rel.source_columns();
    let dest_cols = rel.destination_columns();
    if source_cols.len() != dest_cols.len() {
        return Err(Error::Invalid(format!(
            "relationship `{}` has {} source columns and {} destination columns",
            rel.name,
            source_cols.len(),
            dest_cols.len()
        )));
    }
    let mut parts = Vec::new();
    for (source_col, dest_col) in source_cols.iter().zip(dest_cols.iter()) {
        let dest_sql = column(dialect, dest, dest_col)?;
        let outer_sql = column(dialect, outer, source_col)?;
        parts.push(format!("{dest_alias}.{dest_sql} = {outer_alias}.{outer_sql}"));
    }
    Ok(parts.join(" AND "))
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
    /// Numbers subquery aliases. Kept apart from `param_counter` so Postgres
    /// placeholders stay consecutive.
    alias_counter: usize,
    pub params: Vec<SqlParam>,
    /// A bound value that cannot be sent, reported when the filter finishes compiling.
    invalid_param: Option<Error>,
    pub current_calc_args: Option<FieldMap>,
    /// Declared arguments of the calculation being compiled, which type its parameters.
    current_calc_arguments: &'static [ash_core::ArgumentDef],
    /// Argument values for each calculation the query names, for wherever it appears.
    calc_args: std::collections::HashMap<String, FieldMap>,
    /// Resources whose primary-read filters are being compiled. A read filter that leads
    /// back to its own resource is not applied again inside itself, which would recurse.
    applying_read_filters: Vec<&'static str>,
    /// The query's tenant, which also limits the rows read through relationships.
    tenant: Option<String>,
}

impl<'a, D: SqlDialect> QueryCompiler<'a, D> {
    pub fn new(dialect: &'a D) -> Self {
        Self {
            dialect,
            param_counter: 0,
            alias_counter: 0,
            params: Vec::new(),
            invalid_param: None,
            current_calc_args: None,
            current_calc_arguments: &[],
            calc_args: std::collections::HashMap::new(),
            applying_read_filters: Vec::new(),
            tenant: None,
        }
    }

    /// Compiles `calc` with the query's arguments for it, in a SELECT list or a filter.
    fn compile_calculation(
        &mut self,
        resource: &ResourceDef,
        calc: &CalculationDef,
    ) -> Result<String> {
        let args = self.calc_args.get(calc.name).cloned();
        let outer_args = std::mem::replace(&mut self.current_calc_args, args);
        let outer_arguments = std::mem::replace(&mut self.current_calc_arguments, calc.arguments);
        let compiled = self.compile_expr(resource, &calc.expr);
        self.current_calc_args = outer_args;
        self.current_calc_arguments = outer_arguments;
        compiled
    }

    /// The compiler with `tenant` set, for a write. Reads take the query's tenant.
    pub fn with_tenant(mut self, tenant: Option<&str>) -> Self {
        self.tenant = tenant.map(str::to_string);
        self
    }

    /// `resource`'s table as this statement reaches it: in the tenant's schema for a
    /// context-tenant resource, as AshPostgres prefixes it, or unqualified. Every table a
    /// statement touches goes through here, subqueries included, so none reads the wrong
    /// tenant's rows.
    fn table(&self, resource: &ResourceDef) -> Result<String> {
        let table = ident(self.dialect, resource.table_name())?;
        let schema_tenant = match (resource.multitenancy, self.tenant.as_deref()) {
            (Some(mt), Some(tenant)) if mt.strategy == ash_core::MultitenancyStrategy::Context => {
                tenant
            }
            _ => return Ok(table),
        };
        let schema = ident(self.dialect, schema_tenant)?;
        self.dialect.qualify_table(&schema, &table).ok_or_else(|| {
            Error::Invalid(format!(
                "{} has no schemas to keep the tenants of `{}` apart; use attribute \
                 multitenancy",
                self.dialect.name(),
                resource.name
            ))
        })
    }

    /// What a read of `resource` through a relationship sees, compiled against `alias`:
    /// the query's tenant, and the primary-read filter unless it is already being
    /// compiled further out.
    fn compile_read_filter(
        &mut self,
        resource: &'static ResourceDef,
        alias: &str,
    ) -> Result<Option<String>> {
        let mut parts = Vec::new();
        if let Some(tenant_filter) = resource.tenant_filter(self.tenant.as_deref()) {
            parts.push(self.compile_filter_scoped(resource, &tenant_filter, Some(alias))?);
        }
        if !self.applying_read_filters.contains(&resource.name)
            && let Some(read_filter) = resource.primary_read_filter()
        {
            self.applying_read_filters.push(resource.name);
            let compiled = self.compile_filter_scoped(resource, &read_filter, Some(alias));
            self.applying_read_filters.pop();
            parts.push(compiled?);
        }
        Ok(match parts.len() {
            0 => None,
            1 => parts.pop(),
            _ => Some(format!("({})", parts.join(" AND "))),
        })
    }

    /// Pushes a bound parameter and returns the dialect placeholder (e.g. `?` or `$1`).
    pub fn push_param(&mut self, val: Value) -> String {
        self.param_counter += 1;
        self.params.push(SqlParam::new(val));
        self.dialect.placeholder(self.param_counter)
    }

    fn bind_typed(&mut self, ty: AttrType, val: Value) -> String {
        self.param_counter += 1;
        let placeholder = self.dialect.placeholder(self.param_counter);
        // Stored values are canonical, so compare against the canonical spelling too.
        let val = match &val {
            Value::String(raw) => ash_core::canonical_text(ty, raw).map_or(val, Value::String),
            _ => val,
        };
        if ty == AttrType::Binary {
            if let Value::String(encoded) = &val
                && let Err(err) = ash_core::Binary::parse(encoded)
            {
                self.invalid_param.get_or_insert(err);
            }
            self.params.push(SqlParam::binary(val));
        } else {
            self.params.push(SqlParam::typed(val, ty));
        }
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
            self.compile_calculation(resource, calc)
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
                // Bind with the declared type, so a missing argument is a typed NULL that
                // Postgres can use in `COALESCE` or arithmetic.
                match self.current_calc_arguments.iter().find(|arg| arg.name == *name) {
                    Some(arg) => Ok(self.bind_typed(arg.ty, val)),
                    None => Ok(self.push_param(val)),
                }
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

    fn compile_text_match(
        &mut self,
        resource: &ResourceDef,
        field: &str,
        kind: TextMatch,
        needle: &str,
        scope_alias: Option<&str>,
    ) -> Result<String> {
        self.compile_text_match_with(resource, field, kind, needle, scope_alias, false)
    }

    /// A text filter; `ignore_case` makes it case-insensitive whatever the field's type.
    fn compile_text_match_with(
        &mut self,
        resource: &ResourceDef,
        field: &str,
        kind: TextMatch,
        needle: &str,
        scope_alias: Option<&str>,
        ignore_case: bool,
    ) -> Result<String> {
        let ty = resource
            .attribute(field)
            .map(|attr| attr.ty)
            .or_else(|| resource.calculation(field).map(|calc| calc.ty));
        let case_insensitive = match ty {
            Some(AttrType::CiString) => true,
            Some(AttrType::String | AttrType::Atom { .. }) | None => false,
            Some(other) => {
                return Err(Error::Invalid(format!(
                    "text filters need a string field, but `{field}` on {} is {other:?}",
                    resource.name
                )));
            }
        };
        let case_insensitive = case_insensitive || ignore_case;
        let op = self.compile_operand_scoped(resource, field, scope_alias)?;
        let pattern = self.dialect.text_pattern(kind, needle, case_insensitive);
        let p = self.bind_field(resource, field, Value::String(pattern));
        Ok(self.dialect.render_text_match(&op, &p, case_insensitive))
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
        let sql = self.compile_filter_node(resource, filter, scope_alias)?;
        match self.invalid_param.take() {
            Some(err) => Err(err),
            None => Ok(sql),
        }
    }

    fn compile_filter_node(
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
            Filter::Contains(field, needle) => {
                self.compile_text_match(resource, field, TextMatch::Contains, needle, scope_alias)
            }
            Filter::StartsWith(field, needle) => {
                self.compile_text_match(resource, field, TextMatch::StartsWith, needle, scope_alias)
            }
            Filter::EndsWith(field, needle) => {
                self.compile_text_match(resource, field, TextMatch::EndsWith, needle, scope_alias)
            }
            Filter::Like(field, pattern) => {
                self.compile_text_match(resource, field, TextMatch::Like, pattern, scope_alias)
            }
            Filter::ILike(field, pattern) => self.compile_text_match_with(
                resource,
                field,
                TextMatch::Like,
                pattern,
                scope_alias,
                true,
            ),
            Filter::In(_field, vals) if vals.is_empty() => Ok("0=1".to_string()),
            Filter::In(field, vals) => {
                let op = self.compile_operand_scoped(resource, field, scope_alias)?;
                let ty = resource
                    .attribute(field)
                    .map(|attr| attr.ty)
                    .or_else(|| resource.calculation(field).map(|calc| calc.ty));
                if ty == Some(AttrType::Binary) {
                    // Each value needs decoding, so compare one bound value at a time.
                    let parts: Vec<String> = vals
                        .iter()
                        .map(|val| {
                            let p = self.bind_field(resource, field, val.clone());
                            format!("{op} = {p}")
                        })
                        .collect();
                    return Ok(format!("({})", parts.join(" OR ")));
                }
                let param = self.push_list_param(vals.clone());
                let param = match ty {
                    Some(ty) => self.dialect.cast_list_param(ty, &param),
                    None => param,
                };
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
                self.alias_counter += 1;
                let dest_alias = format!("rel_{}_{}", dest_res.table_name(), self.alias_counter);
                let dest_table = self.table(dest_res)?;
                let outer_scope = scope_alias
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| ident(self.dialect, resource.table_name()).unwrap());
                let join_sql = relationship_equalities(
                    self.dialect,
                    &dest_alias,
                    dest_res,
                    &outer_scope,
                    resource,
                    rel,
                )?;
                let mut inner_sql = self.compile_filter_scoped(dest_res, filter, Some(&dest_alias))?;
                if let Some(read_sql) = self.compile_read_filter(dest_res, &dest_alias)? {
                    inner_sql = format!("({inner_sql} AND {read_sql})");
                }

                match rel.kind {
                    RelKind::ManyToMany => {
                        let through_fn = rel.through.ok_or_else(|| {
                            Error::Invalid(format!(
                                "many_to_many relationship `{}` on `{}` requires through join resource",
                                rel.name, resource.name
                            ))
                        })?;
                        let through_res = through_fn();
                        self.alias_counter += 1;
                        let join_alias = format!(
                            "rel_join_{}_{}",
                            through_res.table_name(),
                            self.alias_counter
                        );
                        let through_table = self.table(through_res)?;
                        let outer_col = column(self.dialect, resource, rel.source_attribute)?;
                        let dest_col = column(self.dialect, dest_res, rel.destination_attribute)?;
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
                        // An archived join row unlinks the records, as it does for loads.
                        if let Some(join_sql) = self.compile_read_filter(through_res, &join_alias)? {
                            inner_sql = format!("{inner_sql} AND {join_sql}");
                        }
                        Ok(format!(
                            "EXISTS (SELECT 1 FROM {dest_table} AS {dest_alias} INNER JOIN {through_table} AS {join_alias} ON {dest_alias}.{dest_col} = {join_alias}.{dest_on_join} WHERE {join_alias}.{source_on_join} = {outer_scope}.{outer_col} AND {inner_sql})"
                        ))
                    }
                    RelKind::BelongsTo | RelKind::HasMany | RelKind::HasOne => Ok(format!(
                        "EXISTS (SELECT 1 FROM {dest_table} AS {dest_alias} WHERE {join_sql} AND {inner_sql})"
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

    /// The aggregate's own filter plus the primary-read filters of the destination and,
    /// for many_to_many, the join resource, each prefixed with ` AND `.
    fn aggregate_conditions(
        &mut self,
        dest: &'static ResourceDef,
        dest_alias: &str,
        agg: &AggregateDef,
        join: Option<(&'static ResourceDef, &str)>,
    ) -> Result<String> {
        let mut sql = self.compile_aggregate_filter(dest_alias, &agg.filter)?;
        if let Some(compiled) = self.compile_read_filter(dest, dest_alias)? {
            sql.push_str(&format!(" AND {compiled}"));
        }
        if let Some((through, join_alias)) = join
            && let Some(compiled) = self.compile_read_filter(through, join_alias)?
        {
            sql.push_str(&format!(" AND {compiled}"));
        }
        Ok(sql)
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
        let dest_table = self.table(dest)?;
        let dest_alias = ident(self.dialect, &format!("_ash_sub_{}", agg.name))?;
        let source_table = ident(self.dialect, resource.table_name())?;
        let join_sql = relationship_equalities(
            self.dialect,
            &dest_alias,
            dest,
            &source_table,
            resource,
            rel,
        )?;
        let source_attr = ident(self.dialect, rel.source_attribute)?;
        let dest_attr = ident(self.dialect, rel.destination_attribute)?;

        match rel.kind {
            RelKind::HasMany | RelKind::BelongsTo | RelKind::HasOne => match &agg.kind {
                AggregateKind::Count => {
                    let mut s = format!(
                        "(SELECT COUNT(*) FROM {dest_table} AS {dest_alias} WHERE {join_sql}"
                    );
                    s.push_str(&self.aggregate_conditions(dest, &dest_alias, agg, None)?);
                    s.push(')');
                    Ok(s)
                }
                AggregateKind::Exists => {
                    let mut s = format!(
                        "(EXISTS (SELECT 1 FROM {dest_table} AS {dest_alias} WHERE {join_sql}"
                    );
                    s.push_str(&self.aggregate_conditions(dest, &dest_alias, agg, None)?);
                    s.push_str("))");
                    Ok(s)
                }
                AggregateKind::First { field } => {
                    let f = ident(self.dialect, field)?;
                    let mut s = format!(
                        "(SELECT {dest_alias}.{f} FROM {dest_table} AS {dest_alias} WHERE {join_sql}"
                    );
                    s.push_str(&self.aggregate_conditions(dest, &dest_alias, agg, None)?);
                    s.push_str(" LIMIT 1)");
                    Ok(s)
                }
                AggregateKind::Sum { field } => {
                    let f = ident(self.dialect, field)?;
                    let mut s = format!(
                        "(SELECT SUM({dest_alias}.{f}) FROM {dest_table} AS {dest_alias} WHERE {join_sql}"
                    );
                    s.push_str(&self.aggregate_conditions(dest, &dest_alias, agg, None)?);
                    s.push(')');
                    // SUM's result type can differ from the column's, so it is cast to the
                    // aggregate's declared type.
                    Ok(self.dialect.cast_expression(agg.ty, &s))
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
                let join_table = self.table(through_def)?;
                let join_alias = ident(self.dialect, &format!("_ash_join_{}", agg.name))?;
                let join = Some((through_def, join_alias.as_str()));
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
                        s.push_str(&self.aggregate_conditions(dest, &dest_alias, agg, join)?);
                        s.push(')');
                        Ok(s)
                    }
                    AggregateKind::Exists => {
                        let mut s = format!(
                            "(EXISTS (SELECT 1 FROM {dest_table} AS {dest_alias} JOIN {join_table} AS {join_alias} ON {dest_alias}.{dest_attr} = {join_alias}.{dest_on_join} WHERE {join_alias}.{source_on_join} = {source_table}.{source_attr}"
                        );
                        s.push_str(&self.aggregate_conditions(dest, &dest_alias, agg, join)?);
                        s.push_str("))");
                        Ok(s)
                    }
                    AggregateKind::First { field } => {
                        let f = ident(self.dialect, field)?;
                        let mut s = format!(
                            "(SELECT {dest_alias}.{f} FROM {dest_table} AS {dest_alias} JOIN {join_table} AS {join_alias} ON {dest_alias}.{dest_attr} = {join_alias}.{dest_on_join} WHERE {join_alias}.{source_on_join} = {source_table}.{source_attr}"
                        );
                        s.push_str(&self.aggregate_conditions(dest, &dest_alias, agg, join)?);
                        s.push_str(" LIMIT 1)");
                        Ok(s)
                    }
                    AggregateKind::Sum { field } => {
                        let f = ident(self.dialect, field)?;
                        let mut s = format!(
                            "(SELECT SUM({dest_alias}.{f}) FROM {dest_table} AS {dest_alias} JOIN {join_table} AS {join_alias} ON {dest_alias}.{dest_attr} = {join_alias}.{dest_on_join} WHERE {join_alias}.{source_on_join} = {source_table}.{source_attr}"
                        );
                        s.push_str(&self.aggregate_conditions(dest, &dest_alias, agg, join)?);
                        s.push(')');
                        // SUM's result type can differ from the column's, so it is cast to the
                        // aggregate's declared type.
                        Ok(self.dialect.cast_expression(agg.ty, &s))
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
        self.calc_args = query.calculation_args.clone();
        self.tenant = query.tenant.clone();
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
            let expr_sql = self.compile_calculation(resource, calc)?;
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
        sql.push_str(&self.table(resource)?);

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

    /// `SELECT COUNT(*)` of the records `query` would return: its filter, and its limit
    /// and offset if it has them. Sort, calculations and aggregates don't change how many.
    pub fn compile_count(&mut self, resource: &ResourceDef, query: &CompiledQuery) -> Result<CompiledSql> {
        self.calc_args = query.calculation_args.clone();
        self.tenant = query.tenant.clone();
        let mut from = self.table(resource)?;
        if let Some(filter) = &query.filter {
            from.push_str(" WHERE ");
            from.push_str(&self.compile_filter(resource, filter)?);
        }
        let sql = if query.limit.is_none() && query.offset.is_none() {
            format!("SELECT COUNT(*) FROM {from}")
        } else {
            let mut page = format!("SELECT 1 FROM {from}");
            if let Some(limit) = query.limit {
                let p = self.push_param(Value::Int(limit as i64));
                page.push_str(&format!(" LIMIT {p}"));
            }
            if let Some(offset) = query.offset {
                if query.limit.is_none() {
                    // SQLite takes an offset only after a limit; -1 is no limit.
                    let p = self.push_param(Value::Int(-1));
                    page.push_str(&format!(" LIMIT {p}"));
                }
                let p = self.push_param(Value::Int(offset as i64));
                page.push_str(&format!(" OFFSET {p}"));
            }
            format!("SELECT COUNT(*) FROM ({page}) AS counted")
        };
        Ok(CompiledSql::new(sql, self.params.clone()))
    }

    /// The type a value compared with, or set to the result of, `expr` takes.
    fn atomic_type(resource: &ResourceDef, expr: &AtomicExpr) -> Option<AttrType> {
        match expr {
            AtomicExpr::Field(name) => resource.attribute(name).map(|attr| attr.ty),
            AtomicExpr::StringLength(_) => Some(AttrType::Integer),
            AtomicExpr::Trim(inner) => Self::atomic_type(resource, inner),
            AtomicExpr::Add(a, b) => Self::atomic_type(resource, a).or_else(|| Self::atomic_type(resource, b)),
            AtomicExpr::Coalesce(items) => items.iter().find_map(|e| Self::atomic_type(resource, e)),
            AtomicExpr::If { then, otherwise, .. } => {
                Self::atomic_type(resource, then).or_else(|| Self::atomic_type(resource, otherwise))
            }
            _ => None,
        }
    }

    /// SQL for an atomic update's expression, over the resource's table. A value takes
    /// `ty`, the type of what it's compared with or set to.
    pub fn compile_atomic_expr(
        &mut self,
        resource: &ResourceDef,
        expr: &AtomicExpr,
        ty: Option<AttrType>,
    ) -> Result<String> {
        let typed = |a: &AtomicExpr, b: &AtomicExpr| {
            Self::atomic_type(resource, a).or_else(|| Self::atomic_type(resource, b))
        };
        Ok(match expr {
            AtomicExpr::Value(value) => match ty {
                Some(ty) => self.bind_typed(ty, value.clone()),
                None => self.push_param(value.clone()),
            },
            AtomicExpr::Field(name) => {
                if resource.attribute(name).is_none() {
                    return Err(Error::Invalid(format!("unknown attribute `{name}` on {}", resource.name)));
                }
                ident(self.dialect, name)?
            }
            AtomicExpr::Add(a, b) => {
                let ty = typed(a, b).or(ty);
                format!("({} + {})", self.compile_atomic_expr(resource, a, ty)?, self.compile_atomic_expr(resource, b, ty)?)
            }
            AtomicExpr::StringLength(e) => format!("char_length({})", self.compile_atomic_expr(resource, e, None)?),
            AtomicExpr::Trim(e) => format!("btrim({})", self.compile_atomic_expr(resource, e, None)?),
            AtomicExpr::Coalesce(items) => {
                let ty = items.iter().find_map(|e| Self::atomic_type(resource, e)).or(ty);
                let parts = items
                    .iter()
                    .map(|e| self.compile_atomic_expr(resource, e, ty))
                    .collect::<Result<Vec<_>>>()?;
                format!("COALESCE({})", parts.join(", "))
            }
            AtomicExpr::If { condition, then, otherwise } => {
                let condition = self.compile_atomic_expr(resource, condition, None)?;
                let ty = typed(then, otherwise).or(ty);
                let then = self.compile_atomic_expr(resource, then, ty)?;
                let otherwise = self.compile_atomic_expr(resource, otherwise, ty)?;
                format!("(CASE WHEN {condition} THEN {then} ELSE {otherwise} END)")
            }
            AtomicExpr::IsNil(e) => format!("({} IS NULL)", self.compile_atomic_expr(resource, e, None)?),
            AtomicExpr::Eq(a, b) | AtomicExpr::Lt(a, b) | AtomicExpr::Gt(a, b) | AtomicExpr::DistinctFrom(a, b) => {
                let op = match expr {
                    AtomicExpr::Eq(..) => "=",
                    AtomicExpr::Lt(..) => "<",
                    AtomicExpr::Gt(..) => ">",
                    _ => "IS DISTINCT FROM",
                };
                let ty = typed(a, b);
                format!("({} {op} {})", self.compile_atomic_expr(resource, a, ty)?, self.compile_atomic_expr(resource, b, ty)?)
            }
            AtomicExpr::In(e, values) => match e.as_ref() {
                AtomicExpr::Field(name) => format!("({})", self.compile_filter(resource, &Filter::In(name.clone(), values.clone()))?),
                other => {
                    if values.is_empty() {
                        "FALSE".to_string()
                    } else {
                        let ty = Self::atomic_type(resource, other);
                        let operand = self.compile_atomic_expr(resource, other, ty)?;
                        let items = values
                            .iter()
                            .map(|v| self.compile_atomic_expr(resource, &AtomicExpr::Value(v.clone()), ty))
                            .collect::<Result<Vec<_>>>()?;
                        format!("({operand} IN ({}))", items.join(", "))
                    }
                }
            },
            AtomicExpr::And(items) | AtomicExpr::Or(items) => {
                if items.is_empty() {
                    return Ok(if matches!(expr, AtomicExpr::And(_)) { "TRUE" } else { "FALSE" }.to_string());
                }
                let joiner = if matches!(expr, AtomicExpr::And(_)) { " AND " } else { " OR " };
                let parts = items
                    .iter()
                    .map(|e| self.compile_atomic_expr(resource, e, None))
                    .collect::<Result<Vec<_>>>()?;
                format!("({})", parts.join(joiner))
            }
            AtomicExpr::Not(e) => format!("(NOT {})", self.compile_atomic_expr(resource, e, None)?),
            AtomicExpr::Filter(filter) => format!("({})", self.compile_filter(resource, filter)?),
        })
    }

    /// An update as one statement, as AshPostgres runs an atomic update:
    ///
    /// ```sql
    /// UPDATE t SET a = s.new_a, ...
    /// FROM (SELECT pk, <new a> AS new_a, ...,
    ///         CASE WHEN <condition> THEN ash_raise_error(<which, and the row>) ... END AS check
    ///       FROM t WHERE <query> LIMIT n FOR UPDATE) AS s
    /// WHERE t.pk = s.pk AND s.check IS NULL
    /// RETURNING t.*
    /// ```
    ///
    /// The subquery locks each record and computes everything from it as it is then, so
    /// no write slips in between; a condition that holds raises its error from the
    /// statement, through the database's `ash_raise_error` function.
    pub fn compile_atomic_update(
        &mut self,
        resource: &ResourceDef,
        query: &CompiledQuery,
        update: &AtomicUpdate,
    ) -> Result<CompiledSql> {
        self.tenant = query.tenant.clone();
        let pk = resource.primary_key().ok_or(Error::NoPrimaryKey(resource.name))?;
        let pk_col = ident(self.dialect, pk.name)?;
        let table = self.table(resource)?;

        let mut items = vec![pk_col.clone()];
        let mut sets = Vec::new();
        for (i, (field, expr)) in update.set.iter().enumerate() {
            let attr = resource
                .attribute(field)
                .ok_or_else(|| Error::Invalid(format!("unknown attribute `{field}` on {}", resource.name)))?;
            let value = self.compile_atomic_expr(resource, expr, Some(attr.ty))?;
            items.push(format!("{} AS \"__ash_set_{i}\"", self.dialect.cast_expression(attr.ty, &value)));
            sets.push(format!("{} = __ash_s.\"__ash_set_{i}\"", ident(self.dialect, field)?));
        }
        if sets.is_empty() {
            return Err(Error::Invalid(format!("an atomic update of {} sets nothing", resource.name)));
        }
        items.extend(self.atomic_check(resource, &update.conditions)?);
        let subquery = self.atomic_subquery(resource, query, &table, &items)?;

        let mut sql = format!(
            "UPDATE {table} AS __ash_t SET {} FROM ({subquery}) AS __ash_s WHERE __ash_t.{pk_col} = __ash_s.{pk_col}",
            sets.join(", ")
        );
        if !update.conditions.is_empty() {
            sql.push_str(" AND __ash_s.\"__ash_check\" IS NULL");
        }
        sql.push_str(" RETURNING __ash_t.*");
        if let Some(err) = self.invalid_param.take() {
            return Err(err);
        }
        Ok(CompiledSql::new(sql, self.params.clone()))
    }

    /// A destroy as one statement, as AshPostgres runs `destroy_query`: like
    /// [`compile_atomic_update`](Self::compile_atomic_update), the records `query` selects
    /// locked and checked against `conditions` in a subquery, then deleted, returning
    /// what they held.
    ///
    /// ```sql
    /// DELETE FROM t AS __ash_t
    /// USING (SELECT pk, CASE WHEN <fails> THEN ash_raise_error(..) ELSE NULL END AS "__ash_check"
    ///        FROM t WHERE <filter> LIMIT n FOR UPDATE) AS __ash_s
    /// WHERE __ash_t.pk = __ash_s.pk AND __ash_s."__ash_check" IS NULL
    /// RETURNING __ash_t.*
    /// ```
    pub fn compile_atomic_destroy(
        &mut self,
        resource: &ResourceDef,
        query: &CompiledQuery,
        conditions: &[AtomicCondition],
    ) -> Result<CompiledSql> {
        self.tenant = query.tenant.clone();
        let pk = resource.primary_key().ok_or(Error::NoPrimaryKey(resource.name))?;
        let pk_col = ident(self.dialect, pk.name)?;
        let table = self.table(resource)?;

        let mut items = vec![pk_col.clone()];
        items.extend(self.atomic_check(resource, conditions)?);
        let subquery = self.atomic_subquery(resource, query, &table, &items)?;

        let mut sql = format!(
            "DELETE FROM {table} AS __ash_t USING ({subquery}) AS __ash_s WHERE __ash_t.{pk_col} = __ash_s.{pk_col}"
        );
        if !conditions.is_empty() {
            sql.push_str(" AND __ash_s.\"__ash_check\" IS NULL");
        }
        sql.push_str(" RETURNING __ash_t.*");
        if let Some(err) = self.invalid_param.take() {
            return Err(err);
        }
        Ok(CompiledSql::new(sql, self.params.clone()))
    }

    /// The `"__ash_check"` column of an atomic statement's subquery: null when no condition
    /// holds, else raised through `ash_raise_error` with the first that does, by index,
    /// and the record's values it reports. `None` without conditions.
    fn atomic_check(&mut self, resource: &ResourceDef, conditions: &[AtomicCondition]) -> Result<Option<String>> {
        if conditions.is_empty() {
            return Ok(None);
        }
        let mut check = String::from("CASE");
        for (i, condition) in conditions.iter().enumerate() {
            let when = self.compile_atomic_expr(resource, &condition.fails_when, None)?;
            let mut row = Vec::new();
            for name in &condition.reports {
                let key = self.push_param(Value::String(name.clone()));
                row.push(format!("{key}::text, {}", ident(self.dialect, name)?));
            }
            check.push_str(&format!(
                " WHEN {when} THEN ash_raise_error(jsonb_build_object('condition', {i}, 'row', jsonb_build_object({})))",
                row.join(", ")
            ));
        }
        check.push_str(" ELSE NULL END");
        Ok(Some(format!("{check} AS \"__ash_check\"")))
    }

    /// The subquery an atomic statement runs over: `items` from the records `query`
    /// selects, locked.
    fn atomic_subquery(
        &mut self,
        resource: &ResourceDef,
        query: &CompiledQuery,
        table: &str,
        items: &[String],
    ) -> Result<String> {
        let mut subquery = format!("SELECT {} FROM {table}", items.join(", "));
        if let Some(filter) = &query.filter {
            subquery.push_str(" WHERE ");
            subquery.push_str(&self.compile_filter(resource, filter)?);
        }
        if let Some(limit) = query.limit {
            let p = self.push_param(Value::Int(limit as i64));
            subquery.push_str(&format!(" LIMIT {p}"));
        }
        subquery.push_str(" FOR UPDATE");
        Ok(subquery)
    }

    pub fn compile_insert(
        &mut self,
        resource: &ResourceDef,
        fields: &FieldMap,
    ) -> Result<CompiledSql> {
        let table = self.table(resource)?;
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
        let table = self.table(resource)?;

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
        let table = self.table(resource)?;

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

    /// Updates `rows`, which all set `columns`, in one statement:
    /// `UPDATE t SET c = v.c, … FROM (VALUES …) AS v(…) WHERE t.pk = v.pk RETURNING t.*`.
    /// For dialects with `VALUES` column aliases and `RETURNING` (Postgres).
    pub fn compile_bulk_update(
        &mut self,
        resource: &ResourceDef,
        columns: &[&str],
        rows: &[(Uuid, &FieldMap)],
    ) -> Result<CompiledSql> {
        let pk = resource
            .primary_key()
            .ok_or(Error::NoPrimaryKey(resource.name))?;
        let table = self.table(resource)?;
        let pk_col = ident(self.dialect, pk.name)?;
        let mut aliases = vec![pk_col.clone()];
        let mut set_clauses = Vec::with_capacity(columns.len());
        let mut types = Vec::with_capacity(columns.len());
        for column in columns {
            let attr = resource
                .attribute(column)
                .ok_or_else(|| Error::Invalid(format!("{} has no attribute {column}", resource.name)))?;
            let col = ident(self.dialect, attr.name)?;
            set_clauses.push(format!("{col} = \"v\".{col}"));
            aliases.push(col);
            types.push((attr.name, attr.ty));
        }
        let mut values = Vec::with_capacity(rows.len());
        for (id, fields) in rows {
            let mut tuple = vec![self.bind_typed(pk.ty, Value::Uuid(*id))];
            for (name, ty) in &types {
                let value = fields.get(*name).cloned().unwrap_or(Value::Null);
                tuple.push(self.bind_typed(*ty, value));
            }
            values.push(format!("({})", tuple.join(", ")));
        }
        let sql = format!(
            "UPDATE {table} SET {} FROM (VALUES {}) AS \"v\" ({}) WHERE {table}.{pk_col} = \"v\".{pk_col} RETURNING {table}.*",
            set_clauses.join(", "),
            values.join(", "),
            aliases.join(", "),
        );
        Ok(CompiledSql::new(sql, self.params.clone()))
    }

    pub fn compile_delete(&mut self, resource: &ResourceDef, id: Uuid) -> Result<CompiledSql> {
        let pk = resource
            .primary_key()
            .ok_or(Error::NoPrimaryKey(resource.name))?;
        let table = self.table(resource)?;
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
        let table = self.table(resource)?;
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
        let if_not_exists = if self.dialect.create_table_if_not_exists() {
            "IF NOT EXISTS "
        } else {
            ""
        };

        for identity in resource.identities {
            let idx_name = ident(
                self.dialect,
                &format!("idx_{}_{}", resource.table_name(), identity.name),
            )?;
            let mut key_cols = Vec::new();
            for key in identity.keys {
                key_cols.push(ident(self.dialect, key)?);
            }
            stmts.push(crate::generator::format_create_index(
                true,
                !if_not_exists.is_empty(),
                &idx_name,
                &table,
                &key_cols.join(", "),
                identity.predicate,
                self.dialect.name(),
                identity.nils_distinct,
                None,
                "",
            ));
        }

        for index in resource.indexes {
            let idx_name = ident(
                self.dialect,
                &format!("idx_{}_{}", resource.table_name(), index.name),
            )?;
            let mut key_cols = Vec::new();
            for key in index.keys {
                key_cols.push(ident(self.dialect, key)?);
            }
            let mut include_cols = Vec::new();
            for column in index.include {
                include_cols.push(ident(self.dialect, column)?);
            }
            stmts.push(crate::generator::format_create_index(
                false,
                !if_not_exists.is_empty(),
                &idx_name,
                &table,
                &key_cols.join(", "),
                index.predicate,
                self.dialect.name(),
                true,
                index.method,
                &include_cols.join(", "),
            ));
        }

        Ok(stmts)
    }
}
