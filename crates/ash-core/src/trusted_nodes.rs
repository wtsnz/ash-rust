//! Developer-owned SQL expressions and read operations.

use crate::{AttrType, DataLayerKind, Error, Expr, FieldMap, Filter, ResourceDef, Result, Sort};

/// A fragment consists only of static SQL, planner-resolved stored fields and typed
/// bound parameters. Never put request values in `Sql` parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FragmentPart {
    Sql(&'static str),
    Field(&'static str),
    Param { name: &'static str, ty: AttrType },
}

/// SQL supported by the explicitly listed data layers. Memory never executes SQL.
/// Field references are local to the expression's resource and use its current alias.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SqlFragment {
    pub layers: &'static [DataLayerKind],
    pub parts: &'static [FragmentPart],
}

impl SqlFragment {
    pub const fn new(layers: &'static [DataLayerKind], parts: &'static [FragmentPart]) -> Self {
        Self { layers, parts }
    }

    /// Bind a predicate in trusted developer code. Client parsers return a different type.
    ///
    /// ```compile_fail
    /// use ash_core::{ClientExpr, Filter, SqlFragment};
    /// let sql = SqlFragment::new(&[], &[]).bind(Default::default());
    /// let _: ClientExpr<Filter> = sql;
    /// ```
    pub fn bind(self, parameters: FieldMap) -> crate::TrustedExpr {
        crate::TrustedExpr::DataLayer(BoundFragment {
            fragment: self,
            parameters,
        })
    }

    pub fn validate(&self, resource: &ResourceDef, layer: DataLayerKind) -> Result<()> {
        if layer == DataLayerKind::Memory || !self.layers.contains(&layer) {
            return Err(Error::UnsupportedPlanNode {
                node: "sql_fragment",
                data_layer: layer,
            });
        }
        for part in self.parts {
            if let FragmentPart::Field(field) = part
                && resource.attribute(field).is_none()
            {
                return Err(Error::Invalid(format!(
                    "fragment field `{field}` must be a stored attribute on {}",
                    resource.name
                )));
            }
        }
        Ok(())
    }

    pub fn validate_parameters(&self, parameters: &FieldMap) -> Result<()> {
        for part in self.parts {
            if let FragmentPart::Param { name, ty } = part {
                let value = parameters.get(*name).ok_or_else(|| {
                    Error::Invalid(format!("missing fragment parameter `{name}`"))
                })?;
                let mut attribute = crate::AttributeDef::required(name, *ty);
                attribute.allow_nil = true;
                crate::pipeline::check_type(&attribute, value)?;
            }
        }
        Ok(())
    }

    pub fn fields(&self, out: &mut Vec<&'static str>) {
        out.extend(self.parts.iter().filter_map(|part| match part {
            FragmentPart::Field(name) => Some(*name),
            _ => None,
        }));
    }
}

/// A trusted predicate with owned parameter values. Only developer code can bind SQL.
#[derive(Clone, PartialEq)]
pub struct BoundFragment {
    fragment: SqlFragment,
    parameters: FieldMap,
}

impl std::fmt::Debug for BoundFragment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parameters = serde_json::to_value(&self.parameters).map_err(|_| std::fmt::Error)?;
        parameters.sort_all_objects();
        f.debug_struct("BoundFragment")
            .field("fragment", &self.fragment)
            .field("parameters", &parameters)
            .finish()
    }
}

impl BoundFragment {
    pub fn fragment(&self) -> &SqlFragment {
        &self.fragment
    }
    pub fn parameters(&self) -> &FieldMap {
        &self.parameters
    }
}

/// Postgres row locking. SQLite and memory reject every variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadLock {
    ForUpdate,
    ForUpdateSkipLocked,
    ForUpdateNowait,
}

/// Select one row per stored-field group, using `sort` to choose the winner.
/// The winner sort is independent of the page direction and ends in a stable key.
#[derive(Clone, Debug, Default)]
pub struct DistinctOn {
    pub fields: Vec<String>,
    pub sort: Vec<Sort>,
}

impl crate::TrustedExpr {
    pub fn into_filter(self) -> Filter {
        match self {
            Self::Filter(filter) => filter,
            Self::DataLayer(fragment) => Filter::DataLayer(Box::new(fragment)),
        }
    }
}

impl Expr {
    /// Validate every branch, even when no rows or only a fallback would be read.
    pub fn validate_data_layer(&self, resource: &ResourceDef, layer: DataLayerKind) -> Result<()> {
        match self {
            Self::Fragment(fragment) => fragment.validate(resource, layer),
            Self::Length(e) | Self::Lower(e) | Self::Upper(e) => {
                e.validate_data_layer(resource, layer)
            }
            Self::Concat(parts) | Self::Coalesce(parts) => parts
                .iter()
                .try_for_each(|e| e.validate_data_layer(resource, layer)),
            Self::Add(a, b)
            | Self::Sub(a, b)
            | Self::Mul(a, b)
            | Self::Div(a, b)
            | Self::Eq(a, b)
            | Self::Ne(a, b)
            | Self::Gt(a, b)
            | Self::Gte(a, b)
            | Self::Lt(a, b)
            | Self::Lte(a, b) => {
                a.validate_data_layer(resource, layer)?;
                b.validate_data_layer(resource, layer)
            }
            Self::IfElse {
                cond,
                then_expr,
                else_expr,
            } => {
                cond.validate_data_layer(resource, layer)?;
                then_expr.validate_data_layer(resource, layer)?;
                else_expr.validate_data_layer(resource, layer)
            }
            _ => Ok(()),
        }
    }
}

impl Filter {
    pub fn validate_data_layer(&self, resource: &ResourceDef, layer: DataLayerKind) -> Result<()> {
        match self {
            Self::DataLayer(node) => node.fragment.validate(resource, layer)?,
            Self::And(parts) | Self::Or(parts) => {
                for part in parts {
                    part.validate_data_layer(resource, layer)?;
                }
            }
            Self::Not(inner) | Self::Projected(inner) | Self::Keyset(inner) => {
                inner.validate_data_layer(resource, layer)?
            }
            Self::Client(input) => input.0.validate_data_layer(resource, layer)?,
            Self::Traversal(node) => {
                let rel = resource
                    .relationship(node.relationship())
                    .ok_or_else(|| Error::Invalid("unknown traversal".into()))?;
                node.filter()
                    .validate_data_layer((rel.destination)(), layer)?;
                if let Some(join) = node.join_filter()
                    && let Some(through) = rel.through
                {
                    join.validate_data_layer(through(), layer)?;
                }
            }
            _ => {}
        }
        let mut fields = Vec::new();
        self.collect_fields(&mut fields);
        for name in fields {
            if let Some(calc) = resource.calculation(name) {
                calc.expr.validate_data_layer(resource, layer)?;
            }
        }
        Ok(())
    }
}

impl crate::CompiledQuery {
    /// Backends call this before acquiring a connection or reading memory tables.
    pub fn validate_data_layer(&self, resource: &ResourceDef, layer: DataLayerKind) -> Result<()> {
        if self.lock.is_some() && layer != DataLayerKind::Postgres {
            return Err(Error::UnsupportedPlanNode {
                node: "row_lock",
                data_layer: layer,
            });
        }
        if let Some(distinct) = &self.distinct_on {
            if layer != DataLayerKind::Postgres {
                return Err(Error::UnsupportedPlanNode {
                    node: "distinct_on",
                    data_layer: layer,
                });
            }
            if distinct.fields.is_empty() {
                return Err(Error::Invalid(
                    "distinct_on needs at least one field".into(),
                ));
            }
            for name in &distinct.fields {
                if resource.attribute(name).is_none() {
                    return Err(Error::Invalid(format!(
                        "distinct_on field `{name}` must be a stored attribute"
                    )));
                }
            }
        }
        if let Some(filter) = &self.filter {
            filter.validate_data_layer(resource, layer)?;
        }
        let mut names: Vec<&str> = self
            .calculations
            .iter()
            .map(String::as_str)
            .chain(self.aggregates.iter().map(String::as_str))
            .chain(self.sort.iter().map(|s| s.field.as_str()))
            .chain(
                self.distinct_on
                    .iter()
                    .flat_map(|d| d.sort.iter().map(|s| s.field.as_str())),
            )
            .collect();
        if let Some(filter) = &self.filter {
            filter.collect_fields(&mut names);
        }
        for name in names {
            if let Some(calc) = resource.calculation(name) {
                calc.expr.validate_data_layer(resource, layer)?;
            }
            if let Some(aggregate) = resource.aggregate(name) {
                let relationship =
                    resource
                        .relationship(aggregate.relationship)
                        .ok_or_else(|| {
                            Error::Invalid(format!(
                                "unknown relationship `{}`",
                                aggregate.relationship
                            ))
                        })?;
                let destination = (relationship.destination)();
                crate::related_read_filter(
                    destination,
                    self.actor.as_ref(),
                    self.tenant.as_deref(),
                )?
                .validate_data_layer(destination, layer)?;
                if let Some(through) = relationship.through {
                    let join = through();
                    crate::related_read_filter(join, self.actor.as_ref(), self.tenant.as_deref())?
                        .validate_data_layer(join, layer)?;
                }
            }
        }
        Ok(())
    }

    pub fn validate_lock(&self, in_transaction: bool) -> Result<()> {
        if self.lock.is_some() && !in_transaction {
            Err(Error::LockRequiresTransaction)
        } else {
            Ok(())
        }
    }
}
