//! Integration fixtures use ordinary actions under explicit system or bound scopes.
//! This module is compiled only into test binaries, never exported by ash-core.
#![cfg(test)]
#![allow(dead_code)]

use ash_core::{
    ActionDef, CompiledQuery, Context, DataLayer, DynamicChangeset, FieldMap, ReadRequest,
    ResourceDef, Result, Runtime, Value, View,
};

static READ_ACTIONS: &[ActionDef] = &[ActionDef::read("fixture_read").primary()];

fn write_action(resource: &ResourceDef, create: bool) -> &'static ActionDef {
    let accepted = Box::leak(
        resource
            .attributes
            .iter()
            .map(|attr| attr.name)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    Box::leak(Box::new(
        if create {
            ActionDef::create("fixture_create")
        } else {
            ActionDef::update("fixture_update")
        }
        .accept(accepted),
    ))
}

pub(crate) fn request(query: &CompiledQuery) -> ReadRequest {
    ReadRequest {
        lock: query.lock,
        distinct_on: query.distinct_on.clone(),
        filter: query.filter.clone(),
        sort: query.sort.clone(),
        select: query.select.clone(),
        calculations: query.calculations.clone(),
        calculation_args: query.calculation_args.clone(),
        aggregates: query.aggregates.clone(),
        limit: query.limit,
        offset: query.offset,
        keyset: None,
    }
}

pub(crate) async fn stored<D: DataLayer>(
    ctx: &Context<D>,
    resource: &ResourceDef,
    query: &CompiledQuery,
) -> Result<Vec<FieldMap>> {
    let resource = ResourceDef {
        actions: READ_ACTIONS,
        ..*resource
    };
    ctx.access()
        .read(&resource, "fixture_read", &FieldMap::new(), request(query))
        .await
        .map(|rows| rows.into_iter().map(View::into_fields).collect())
}

pub(crate) trait Fixtures: DataLayer + Clone + Sized {
    async fn fixture_create(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Value,
        mut fields: FieldMap,
    ) -> Result<FieldMap> {
        let resource = Box::leak(Box::new(*resource));
        if let Some(pk) = resource.primary_key() {
            fields.insert(pk.name.to_owned(), id);
        }
        let ctx = Runtime::new(self.clone())
            .system()
            .bind(None, tenant.map(str::to_owned));
        DynamicChangeset::for_create(&ctx, resource, write_action(resource, true), fields)?
            .commit(&ctx)
            .await
            .map(View::into_fields)
    }
    async fn fixture_update(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Value,
        fields: FieldMap,
    ) -> Result<FieldMap> {
        let resource = Box::leak(Box::new(*resource));
        let ctx = Runtime::new(self.clone())
            .system()
            .bind(None, tenant.map(str::to_owned));
        let pk = resource.primary_key().unwrap().name;
        let query = CompiledQuery {
            filter: Some(ash_core::Filter::eq(pk, id)),
            ..Default::default()
        };
        let existing = stored(&ctx, resource, &query)
            .await?
            .into_iter()
            .next()
            .ok_or(ash_core::Error::NotFound)?;
        DynamicChangeset::for_update(
            &ctx,
            resource,
            write_action(resource, false),
            existing,
            fields,
        )?
        .commit(&ctx)
        .await
        .map(View::into_fields)
    }
    async fn fixture_destroy(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Value,
    ) -> Result<()> {
        let resource = Box::leak(Box::new(*resource));
        let ctx = Runtime::new(self.clone())
            .system()
            .bind(None, tenant.map(str::to_owned));
        let query = CompiledQuery {
            filter: Some(ash_core::Filter::eq(
                resource.primary_key().unwrap().name,
                id,
            )),
            ..Default::default()
        };
        let existing = stored(&ctx, resource, &query)
            .await?
            .into_iter()
            .next()
            .ok_or(ash_core::Error::NotFound)?;
        static DESTROY: ActionDef = ActionDef::destroy("fixture_destroy");
        DynamicChangeset::for_destroy(&ctx, resource, &DESTROY, existing)?
            .commit(&ctx)
            .await
            .map(|_| ())
    }
    async fn fixture_run_query(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
    ) -> Result<Vec<FieldMap>> {
        let ctx = Runtime::new(self.clone()).bind(query.actor.clone(), query.tenant.clone());
        stored(&ctx, resource, query).await
    }
    async fn fixture_count(&self, resource: &ResourceDef, query: &CompiledQuery) -> Result<usize> {
        let resource = ResourceDef {
            actions: READ_ACTIONS,
            ..*resource
        };
        let ctx = Runtime::new(self.clone()).bind(query.actor.clone(), query.tenant.clone());
        ctx.access()
            .count(&resource, "fixture_read", &FieldMap::new(), request(query))
            .await
    }
    async fn fixture_bulk_create(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        rows: Vec<(Value, FieldMap)>,
    ) -> Result<Vec<FieldMap>> {
        let mut created = Vec::new();
        for (id, fields) in rows {
            created.push(self.fixture_create(resource, tenant, id, fields).await?);
        }
        Ok(created)
    }
    async fn fixture_upsert(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Value,
        mut fields: FieldMap,
        identity: &ash_core::IdentityDef,
        updates: &[String],
    ) -> Result<FieldMap> {
        let resource = Box::leak(Box::new(*resource));
        if let Some(pk) = resource.primary_key() {
            fields.insert(pk.name.to_owned(), id);
        }
        let ctx = Runtime::new(self.clone())
            .system()
            .bind(None, tenant.map(str::to_owned));
        let updates: Vec<_> = updates.iter().map(String::as_str).collect();
        DynamicChangeset::for_create(&ctx, resource, write_action(resource, true), fields)?
            .with_upsert(identity.name, &updates)
            .commit(&ctx)
            .await
            .map(View::into_fields)
    }
}
impl<D: DataLayer + Clone> Fixtures for D {}

pub(crate) async fn update<D: DataLayer>(
    ctx: &Context<D>,
    resource: &ResourceDef,
    id: Value,
    fields: FieldMap,
) -> Result<FieldMap> {
    let resource = Box::leak(Box::new(*resource));
    let query = CompiledQuery {
        filter: Some(ash_core::Filter::eq(
            resource.primary_key().unwrap().name,
            id,
        )),
        ..Default::default()
    };
    let existing = stored(ctx, resource, &query)
        .await?
        .into_iter()
        .next()
        .ok_or(ash_core::Error::NotFound)?;
    DynamicChangeset::for_update(
        ctx,
        resource,
        write_action(resource, false),
        existing,
        fields,
    )?
    .commit(ctx)
    .await
    .map(View::into_fields)
}

pub(crate) async fn destroy<D: DataLayer>(
    ctx: &Context<D>,
    resource: &ResourceDef,
    id: Value,
) -> Result<()> {
    let resource = Box::leak(Box::new(*resource));
    let query = CompiledQuery {
        filter: Some(ash_core::Filter::eq(
            resource.primary_key().unwrap().name,
            id,
        )),
        ..Default::default()
    };
    let existing = stored(ctx, resource, &query)
        .await?
        .into_iter()
        .next()
        .ok_or(ash_core::Error::NotFound)?;
    static DESTROY: ActionDef = ActionDef::destroy("fixture_destroy");
    DynamicChangeset::for_destroy(ctx, resource, &DESTROY, existing)?
        .commit(ctx)
        .await
        .map(|_| ())
}
