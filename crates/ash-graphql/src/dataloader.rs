use std::collections::HashMap;
use std::sync::Arc;
use ash_core::{CompiledQuery, Context, DataLayer, FieldMap, Filter, ResourceDef, Value};
use async_graphql::dataloader::Loader;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BelongsToKey {
    pub dest_resource: &'static str,
    pub dest_attr: &'static str,
    pub foreign_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HasManyKey {
    pub dest_resource: &'static str,
    pub dest_attr: &'static str,
    pub source_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ManyToManyKey {
    pub join_resource: &'static str,
    pub dest_resource: &'static str,
    pub source_attr_on_join: &'static str,
    pub dest_attr_on_join: &'static str,
    pub source_id: Uuid,
}

/// Generic batch loader resolving relationships in constant time (O(1) queries instead of O(N)).
pub struct AshBatchLoader<D> {
    pub ctx: Context<D>,
    pub resources: Vec<&'static ResourceDef>,
}

impl<D: Clone> AshBatchLoader<D> {
    pub fn new(ctx: Context<D>, resources: &[&'static ResourceDef]) -> Self {
        Self {
            ctx,
            resources: resources.to_vec(),
        }
    }

    fn find_resource(&self, name: &str) -> Option<&'static ResourceDef> {
        self.resources.iter().copied().find(|r| r.name == name)
    }
}

impl<D: DataLayer + Clone + 'static> Loader<BelongsToKey> for AshBatchLoader<D> {
    type Value = FieldMap;
    type Error = Arc<async_graphql::Error>;

    async fn load(
        &self,
        keys: &[BelongsToKey],
    ) -> Result<HashMap<BelongsToKey, Self::Value>, Self::Error> {
        let mut results = HashMap::new();
        if keys.is_empty() {
            return Ok(results);
        }

        // Group keys by (dest_resource, dest_attr)
        let mut groups: HashMap<(&'static str, &'static str), Vec<Uuid>> = HashMap::new();
        for k in keys {
            groups
                .entry((k.dest_resource, k.dest_attr))
                .or_default()
                .push(k.foreign_id);
        }

        for ((dest_res_name, dest_attr), ids) in groups {
            let res = self.find_resource(dest_res_name).ok_or_else(|| {
                Arc::new(async_graphql::Error::new(format!(
                    "Resource `{dest_res_name}` not found in DataLoader"
                )))
            })?;

            let id_values: Vec<Value> = ids.into_iter().map(Value::Uuid).collect();
            let query = CompiledQuery {
                filter: Some(Filter::in_list(dest_attr, id_values)),
                tenant: self.ctx.tenant.clone(),
                ..CompiledQuery::default()
            };

            let records = self.ctx.data.run_query(res, &query).await.map_err(|e| {
                Arc::new(async_graphql::Error::new(format!(
                    "DataLoader query error: {e}"
                )))
            })?;

            let mut by_attr: HashMap<Uuid, FieldMap> = HashMap::new();
            for rec in records {
                if let Some(id_val) = rec.get(dest_attr).and_then(|v| v.as_uuid()) {
                    by_attr.insert(id_val, rec);
                }
            }

            for k in keys {
                if k.dest_resource == dest_res_name
                    && k.dest_attr == dest_attr
                    && let Some(rec) = by_attr.get(&k.foreign_id)
                {
                    results.insert(k.clone(), rec.clone());
                }
            }
        }

        Ok(results)
    }
}

impl<D: DataLayer + Clone + 'static> Loader<HasManyKey> for AshBatchLoader<D> {
    type Value = Vec<FieldMap>;
    type Error = Arc<async_graphql::Error>;

    async fn load(
        &self,
        keys: &[HasManyKey],
    ) -> Result<HashMap<HasManyKey, Self::Value>, Self::Error> {
        let mut results = HashMap::new();
        if keys.is_empty() {
            return Ok(results);
        }

        let mut groups: HashMap<(&'static str, &'static str), Vec<Uuid>> = HashMap::new();
        for k in keys {
            groups
                .entry((k.dest_resource, k.dest_attr))
                .or_default()
                .push(k.source_id);
        }

        for ((dest_res_name, dest_attr), ids) in groups {
            let res = self.find_resource(dest_res_name).ok_or_else(|| {
                Arc::new(async_graphql::Error::new(format!(
                    "Resource `{dest_res_name}` not found in DataLoader"
                )))
            })?;

            let id_values: Vec<Value> = ids.into_iter().map(Value::Uuid).collect();
            let query = CompiledQuery {
                filter: Some(Filter::in_list(dest_attr, id_values)),
                tenant: self.ctx.tenant.clone(),
                ..CompiledQuery::default()
            };

            let records = self.ctx.data.run_query(res, &query).await.map_err(|e| {
                Arc::new(async_graphql::Error::new(format!(
                    "DataLoader query error: {e}"
                )))
            })?;

            let mut by_parent: HashMap<Uuid, Vec<FieldMap>> = HashMap::new();
            for rec in records {
                if let Some(parent_id) = rec.get(dest_attr).and_then(|v| v.as_uuid()) {
                    by_parent.entry(parent_id).or_default().push(rec);
                }
            }

            for k in keys {
                if k.dest_resource == dest_res_name && k.dest_attr == dest_attr {
                    let matching = by_parent.remove(&k.source_id).unwrap_or_default();
                    results.insert(k.clone(), matching);
                }
            }
        }

        Ok(results)
    }
}

impl<D: DataLayer + Clone + 'static> Loader<ManyToManyKey> for AshBatchLoader<D> {
    type Value = Vec<FieldMap>;
    type Error = Arc<async_graphql::Error>;

    async fn load(
        &self,
        keys: &[ManyToManyKey],
    ) -> Result<HashMap<ManyToManyKey, Self::Value>, Self::Error> {
        let mut results = HashMap::new();
        if keys.is_empty() {
            return Ok(results);
        }

        for k in keys {
            let join_res = self.find_resource(k.join_resource).ok_or_else(|| {
                Arc::new(async_graphql::Error::new(format!(
                    "Join resource `{}` not found in DataLoader",
                    k.join_resource
                )))
            })?;
            let dest_res = self.find_resource(k.dest_resource).ok_or_else(|| {
                Arc::new(async_graphql::Error::new(format!(
                    "Dest resource `{}` not found in DataLoader",
                    k.dest_resource
                )))
            })?;

            // 1. Query join table
            let join_query = CompiledQuery {
                filter: Some(Filter::eq(
                    k.source_attr_on_join,
                    Value::Uuid(k.source_id),
                )),
                tenant: self.ctx.tenant.clone(),
                ..CompiledQuery::default()
            };
            let join_rows = self
                .ctx
                .data
                .run_query(join_res, &join_query)
                .await
                .map_err(|e| Arc::new(async_graphql::Error::new(e.to_string())))?;

            let dest_ids: Vec<Value> = join_rows
                .into_iter()
                .filter_map(|r| r.get(k.dest_attr_on_join).cloned())
                .collect();

            if dest_ids.is_empty() {
                results.insert(k.clone(), Vec::new());
                continue;
            }

            // 2. Query destination table
            let pk = dest_res
                .attributes
                .iter()
                .find(|a| a.primary_key)
                .map(|a| a.name)
                .unwrap_or("id");

            let dest_query = CompiledQuery {
                filter: Some(Filter::in_list(pk, dest_ids)),
                tenant: self.ctx.tenant.clone(),
                ..CompiledQuery::default()
            };

            let dest_records = self
                .ctx
                .data
                .run_query(dest_res, &dest_query)
                .await
                .map_err(|e| Arc::new(async_graphql::Error::new(e.to_string())))?;

            results.insert(k.clone(), dest_records);
        }

        Ok(results)
    }
}
