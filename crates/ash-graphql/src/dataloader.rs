use ash_core::{Context, DataLayer, FieldMap, RelationshipDef, ResourceDef, Value};
use async_graphql::dataloader::Loader;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::redact::key_value;

/// One source record's relationship, batched by [`AshBatchLoader`].
#[derive(Clone, Debug)]
pub struct RelatedKey {
    pub resource: &'static ResourceDef,
    pub relationship: &'static str,
    /// The source record's key columns, which are all a load needs from it.
    pub source: Vec<(&'static str, Value)>,
}

impl RelatedKey {
    pub fn new(
        resource: &'static ResourceDef,
        relationship: &'static RelationshipDef,
        source: &FieldMap,
    ) -> Self {
        Self {
            resource,
            relationship: relationship.name,
            source: relationship
                .source_columns()
                .into_iter()
                .map(|column| (column, key_value(source, column)))
                .collect(),
        }
    }

    fn source_record(&self) -> FieldMap {
        self.source
            .iter()
            .map(|(column, value)| (column.to_string(), value.clone()))
            .collect()
    }
}

impl PartialEq for RelatedKey {
    fn eq(&self, other: &Self) -> bool {
        self.resource.name == other.resource.name
            && self.relationship == other.relationship
            && self.source == other.source
    }
}

impl Eq for RelatedKey {}

impl Hash for RelatedKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.resource.name.hash(state);
        self.relationship.hash(state);
        self.source.hash(state);
    }
}

/// Batches relationship loads: every key of one relationship in a tick is loaded with a
/// single [`ash_core::load_related`], which reads the destination (and any join
/// resource) as `ctx` would, through its read policies, filters and tenant.
pub struct AshBatchLoader<D> {
    pub ctx: Context<D>,
}

impl<D> AshBatchLoader<D> {
    pub fn new(ctx: Context<D>) -> Self {
        Self { ctx }
    }

    /// Whether this loader reads as `ctx` does: the same actor and tenant, on the same
    /// data layer. A loader serving another context must not answer for this one.
    pub fn serves(&self, ctx: &Context<D>) -> bool {
        self.ctx.actor == ctx.actor
            && self.ctx.tenant == ctx.tenant
            && Arc::ptr_eq(&self.ctx.data, &ctx.data)
    }
}

impl<D: DataLayer + Clone + 'static> Loader<RelatedKey> for AshBatchLoader<D> {
    type Value = Vec<FieldMap>;
    type Error = Arc<async_graphql::Error>;

    async fn load(
        &self,
        keys: &[RelatedKey],
    ) -> Result<HashMap<RelatedKey, Self::Value>, Self::Error> {
        let mut groups: HashMap<(&'static str, &'static str), Vec<&RelatedKey>> = HashMap::new();
        for key in keys {
            groups
                .entry((key.resource.name, key.relationship))
                .or_default()
                .push(key);
        }

        let mut results = HashMap::with_capacity(keys.len());
        for group in groups.into_values() {
            let sources: Vec<FieldMap> = group.iter().map(|key| key.source_record()).collect();
            let related = ash_core::load_related(
                &self.ctx,
                group[0].resource,
                group[0].relationship,
                &sources,
            )
            .await
            .map_err(|e| Arc::new(async_graphql::Error::new(e.to_string())))?;
            for (key, rows) in group.into_iter().zip(related) {
                results.insert(key.clone(), rows);
            }
        }
        Ok(results)
    }
}
