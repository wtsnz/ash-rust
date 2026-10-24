//! Expression provenance and resolved relationship reads shared by all data layers.

use crate::{Actor, Error, FieldMap, Filter, ResourceDef, Result, Sort};

/// An expression supplied by a client. Hidden fields read as null.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientExpr<T>(pub(crate) T);

impl<T> ClientExpr<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }
}

impl ClientExpr<Filter> {
    /// Retain provenance until the scope-bound read planner resolves the expression.
    pub fn into_filter(self) -> Filter {
        Filter::Client(Box::new(self))
    }
}

/// Server expressions may use hidden values. Future backend-tagged expressions belong
/// here, so client parsers never acquire a backend expression constructor.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum TrustedExpr {
    Filter(Filter),
}

/// A resolved relationship hop. Its destination and optional join conditions already
/// include read preparations and attribute tenancy. Client hops also carry destination
/// and join row authorization; trusted preparation hops retain their server semantics.
#[derive(Clone, Debug, PartialEq)]
pub struct Traversal {
    pub(crate) relationship: String,
    pub(crate) filter: Box<Filter>,
    pub(crate) join: Option<Box<Filter>>,
    pub(crate) actor: Option<Actor>,
}

impl Traversal {
    pub fn actor(&self) -> Option<&Actor> {
        self.actor.as_ref()
    }

    pub fn relationship(&self) -> &str {
        &self.relationship
    }
    pub fn filter(&self) -> &Filter {
        &self.filter
    }
    pub fn join_filter(&self) -> Option<&Filter> {
        self.join.as_deref()
    }
}

/// Normalize trusted and client expressions through one planner. Backend compilers
/// consume `Traversal` conditions without adding their own read authorization.
pub fn plan_filter(
    resource: &ResourceDef,
    actor: Option<&Actor>,
    tenant: Option<&str>,
    expression: TrustedExpr,
) -> Result<Filter> {
    let TrustedExpr::Filter(filter) = expression;
    Planner {
        actor,
        tenant,
        applying: Vec::new(),
        reads: std::collections::HashMap::new(),
        depth: 0,
    }
    .filter(resource, filter)
}

/// The condition for a destination or join's default read, also used by aggregates.
pub fn related_read_filter(
    resource: &ResourceDef,
    actor: Option<&Actor>,
    tenant: Option<&str>,
) -> Result<Filter> {
    Planner {
        actor,
        tenant,
        applying: Vec::new(),
        reads: std::collections::HashMap::new(),
        depth: 0,
    }
    .read(resource, Origin::Client)
}

pub fn plan_client_sort(
    resource: &ResourceDef,
    actor: Option<&Actor>,
    sorts: ClientExpr<Vec<Sort>>,
) -> Result<Vec<Sort>> {
    crate::policy::guard_input_sort(resource, actor, sorts.0)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Origin {
    Client,
    Trusted,
}

struct Planner<'a> {
    actor: Option<&'a Actor>,
    tenant: Option<&'a str>,
    applying: Vec<&'static str>,
    reads: std::collections::HashMap<(usize, usize, Origin), Filter>,
    depth: usize,
}

impl Planner<'_> {
    fn read(&mut self, resource: &ResourceDef, origin: Origin) -> Result<Filter> {
        // Actor and tenant are fixed for this planner. Cache complete default reads,
        // but never a preparation whose recursion guard depends on an outer read.
        let key = (
            resource as *const ResourceDef as usize,
            resource.default_read() as *const crate::ActionDef as usize,
            origin,
        );
        let cacheable = self.applying.is_empty();
        if cacheable && let Some(filter) = self.reads.get(&key) {
            return Ok(filter.clone());
        }
        let filter = self.read_scope(resource, origin)?;
        if cacheable {
            self.reads.insert(key, filter.clone());
        }
        Ok(filter)
    }

    fn read_scope(&mut self, resource: &ResourceDef, origin: Origin) -> Result<Filter> {
        let mut parts = Vec::new();
        let (tenant, _) =
            crate::pipeline::apply_tenant_scope(resource, None, self.tenant.map(str::to_owned))?;
        parts.extend(tenant);
        if origin == Origin::Client {
            match crate::compile_read_filter(resource, resource.default_read(), self.actor) {
                Ok(policy) => parts.extend(policy),
                Err(Error::Forbidden) => return Ok(Filter::False),
                Err(error) => return Err(error),
            }
        }
        // A preparation leading back to its own resource applies only once.
        if !self.applying.contains(&resource.name) {
            self.applying.push(resource.name);
            let prepared = resource
                .primary_read_filter()
                .map(|filter| self.filter(resource, filter))
                .transpose();
            self.applying.pop();
            parts.extend(prepared?);
        }
        Ok(Filter::and(parts))
    }

    fn filter(&mut self, resource: &ResourceDef, filter: Filter) -> Result<Filter> {
        self.filter_at(resource, filter, true, Origin::Trusted)
    }

    fn filter_at(
        &mut self,
        resource: &ResourceDef,
        filter: Filter,
        positive: bool,
        origin: Origin,
    ) -> Result<Filter> {
        if self.depth >= 64 {
            return Err(Error::Invalid(
                "recursive read plan exceeds 64 expression levels".into(),
            ));
        }
        self.depth += 1;
        let result = self.filter_inner(resource, filter, positive, origin);
        self.depth -= 1;
        result
    }

    fn filter_inner(
        &mut self,
        resource: &ResourceDef,
        filter: Filter,
        positive: bool,
        origin: Origin,
    ) -> Result<Filter> {
        Ok(match filter {
            Filter::Client(input) => {
                let guarded =
                    crate::policy::guard_client_fields(resource, self.actor, input.0, positive)?;
                self.filter_at(resource, guarded, positive, Origin::Client)?
            }
            Filter::And(parts) => Filter::and(
                parts
                    .into_iter()
                    .map(|part| self.filter_at(resource, part, positive, origin))
                    .collect::<Result<Vec<_>>>()?,
            ),
            Filter::Or(parts) => Filter::or(
                parts
                    .into_iter()
                    .map(|part| self.filter_at(resource, part, positive, origin))
                    .collect::<Result<Vec<_>>>()?,
            ),
            Filter::Not(inner) => !self.filter_at(resource, *inner, !positive, origin)?,
            Filter::Related {
                relationship,
                filter,
            } => {
                let rel = resource.relationship(&relationship).ok_or_else(|| {
                    Error::Invalid(format!(
                        "unknown relationship `{relationship}` on {}",
                        resource.name
                    ))
                })?;
                let dest = (rel.destination)();
                let inner = self.filter_at(dest, *filter, true, origin)?;
                let scoped = self.read(dest, origin)?;
                let join = rel
                    .through
                    .map(|through| self.read(through(), origin).map(Box::new))
                    .transpose()?;
                Filter::Traversal(Box::new(Traversal {
                    relationship,
                    filter: Box::new(Filter::and([scoped, inner])),
                    join,
                    actor: self.actor.cloned(),
                }))
            }
            // Resolved nodes are opaque and can be carried through composed reads.
            other => other,
        })
    }
}

/// Client action arguments that name attributes are equality comparisons, not trusted
/// preparation predicates. `FilterWithArgs` remains trusted application code.
pub(crate) fn argument_filter(resource: &ResourceDef, arguments: &FieldMap) -> Filter {
    ClientExpr::new(Filter::and(
        arguments
            .iter()
            .filter(|(name, _)| resource.attribute(name).is_some())
            .map(|(name, value)| Filter::eq(name, value.clone())),
    ))
    .into_filter()
}
