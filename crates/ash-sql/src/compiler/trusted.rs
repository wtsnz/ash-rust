use super::*;
use ash_core::{DataLayerKind, FragmentPart, ReadLock, SqlFragment};

impl<D: SqlDialect> QueryCompiler<'_, D> {
    pub(super) fn data_layer_kind(&self) -> DataLayerKind {
        match self.dialect.name() {
            "postgres" => DataLayerKind::Postgres,
            "sqlite" => DataLayerKind::Sqlite,
            name => DataLayerKind::Custom(name),
        }
    }

    pub(super) fn compile_fragment(
        &mut self,
        resource: &ResourceDef,
        fragment: &SqlFragment,
        parameters: &FieldMap,
        alias: Option<&str>,
    ) -> Result<String> {
        fragment.validate(resource, self.data_layer_kind())?;
        fragment.validate_parameters(parameters)?;
        let mut sql = String::from("(");
        for part in fragment.parts {
            match part {
                FragmentPart::Sql(text) => sql.push_str(text),
                FragmentPart::Field(field) => {
                    let col = column(self.dialect, resource, field)?;
                    if let Some(alias) = alias {
                        sql.push_str(alias);
                        sql.push('.');
                    }
                    sql.push_str(&col);
                }
                FragmentPart::Param { name, ty } => {
                    let value = parameters.get(*name).cloned().ok_or_else(|| {
                        Error::Invalid(format!("missing fragment parameter `{name}`"))
                    })?;
                    sql.push_str(&self.bind_typed(*ty, value));
                }
            }
        }
        sql.push(')');
        Ok(sql)
    }

    pub(super) fn append_lock(&self, query: &CompiledQuery, sql: &mut String) -> Result<()> {
        if let Some(lock) = query.lock {
            if query.distinct_on.is_some() {
                return Err(Error::Invalid(
                    "row locks cannot be combined with distinct_on".into(),
                ));
            }
            sql.push_str(match lock {
                ReadLock::ForUpdate => " FOR UPDATE",
                ReadLock::ForUpdateSkipLocked => " FOR UPDATE SKIP LOCKED",
                ReadLock::ForUpdateNowait => " FOR UPDATE NOWAIT",
            });
        }
        Ok(())
    }

    /// Deduplicate before applying a sealed cursor. Filtering first could promote a
    /// group's losing row on the next page. Winner ordering never reverses.
    pub(super) fn compile_distinct(
        &mut self,
        resource: &ResourceDef,
        query: &CompiledQuery,
    ) -> Result<CompiledSql> {
        query.validate_data_layer(resource, self.data_layer_kind())?;
        if query.lock.is_some() {
            return Err(Error::Invalid(
                "row locks cannot be combined with distinct_on".into(),
            ));
        }
        let distinct = query.distinct_on.as_ref().expect("distinct query");
        let mut inner = query.clone();
        inner.distinct_on = None;
        inner.limit = None;
        inner.offset = None;
        inner.select = None;
        let (base, boundary) = split_keyset(query.filter.clone().unwrap_or(Filter::True))?;
        inner.filter = Some(base);
        inner.sort = distinct
            .fields
            .iter()
            .map(|field| Sort {
                field: field.clone(),
                ..Default::default()
            })
            .collect();
        let winner_sort = if distinct.sort.is_empty() {
            &query.sort
        } else {
            &distinct.sort
        };
        // A guarded group field can differ between rows in the same raw group.
        // Keep the entire winner sort after Postgres's required group prefix.
        inner.sort.extend(winner_sort.iter().cloned());
        inner.sort = ash_core::keyset_sort(resource, inner.sort);
        let per_key = self.per_key.is_some();
        let previous = std::mem::replace(&mut self.force_inline_aggregates, true);
        let compiled = self.compile_select_internal(resource, &inner, None);
        self.force_inline_aggregates = previous;
        let compiled = compiled?;
        let fields = distinct
            .fields
            .iter()
            .map(|name| column(self.dialect, resource, name))
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        let selected =
            compiled
                .sql
                .replacen("SELECT ", &format!("SELECT DISTINCT ON ({fields}) "), 1);
        let table = ident(self.dialect, resource.table_name())?;
        let items = if per_key {
            let order = self.compile_sort(resource, &query.sort)?;
            format!("*, row_number() OVER ({}) AS \"__ash_row\"", order.trim())
        } else {
            "*".into()
        };
        let mut sql = format!("SELECT {items} FROM ({selected}) AS {table}");
        if boundary != Filter::True {
            sql.push_str(" WHERE ");
            sql.push_str(&self.compile_filter(resource, &boundary)?);
        }
        sql.push_str(&self.compile_sort(resource, &query.sort)?);
        if let Some(limit) = query.limit {
            let p = self.push_param(Value::Int(limit as i64));
            sql.push_str(&format!(" LIMIT {p}"));
        }
        if let Some(offset) = query.offset {
            let p = self.push_param(Value::Int(offset as i64));
            sql.push_str(&format!(" OFFSET {p}"));
        }
        Ok(CompiledSql::new(sql, self.params.clone()))
    }
}

fn split_keyset(filter: Filter) -> Result<(Filter, Filter)> {
    match filter {
        Filter::Keyset(inner) => Ok((Filter::True, *inner)),
        Filter::And(parts) => {
            let (mut base, mut boundary) = (Vec::new(), Vec::new());
            for part in parts {
                let (b, p) = split_keyset(part)?;
                base.push(b);
                boundary.push(p);
            }
            Ok((Filter::and(base), Filter::and(boundary)))
        }
        Filter::Or(parts) => {
            for part in &parts {
                if split_keyset(part.clone())?.1 != Filter::True {
                    return Err(Error::Invalid(
                        "distinct_on page boundaries must be conjunctive".into(),
                    ));
                }
            }
            Ok((Filter::Or(parts), Filter::True))
        }
        Filter::Not(inner) => {
            if split_keyset(*inner.clone())?.1 != Filter::True {
                return Err(Error::Invalid(
                    "distinct_on page boundaries cannot be negated".into(),
                ));
            }
            Ok((Filter::Not(inner), Filter::True))
        }
        other => Ok((other, Filter::True)),
    }
}
