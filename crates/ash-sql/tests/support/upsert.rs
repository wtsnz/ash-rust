//! Obtain compiler inputs by running core's actual conflict planner.
#![cfg(test)]
use ash_core::{DataLayer, Error, FieldMap, Result};
use ash_sql::{CompiledSql, PostgresDialect, QueryCompiler};
use std::sync::{Arc, Mutex};

struct Capture(Arc<Mutex<Option<CompiledSql>>>);
impl DataLayer for Capture {
    async fn create(&self, _: &ash_core::CreatePlan<'_>) -> Result<FieldMap> {
        Err(Error::Forbidden)
    }
    async fn update(&self, _: &ash_core::UpdatePlan<'_>) -> Result<FieldMap> {
        Err(Error::Forbidden)
    }
    async fn destroy(&self, _: &ash_core::DestroyPlan<'_>) -> Result<()> {
        Err(Error::Forbidden)
    }
    async fn run_query(&self, _: &ash_core::QueryPlan<'_>) -> Result<Vec<FieldMap>> {
        Err(Error::Forbidden)
    }
    async fn upsert(&self, plan: &ash_core::UpsertPlan<'_>) -> Result<FieldMap> {
        *self.0.lock().unwrap() = Some(QueryCompiler::new(&PostgresDialect).compile_upsert(plan)?);
        Ok(plan.fields().clone())
    }
}

pub(crate) async fn compile(
    resource: &ash_core::ResourceDef,
    tenant: Option<&str>,
    fields: &FieldMap,
    identity: &'static str,
    updates: &[&str],
) -> CompiledSql {
    let captured = Arc::new(Mutex::new(None));
    let ctx = ash_core::Runtime::new(Capture(Arc::clone(&captured)))
        .system()
        .bind(None, tenant.map(str::to_owned));
    let resource = Box::leak(Box::new(*resource));
    let accepted = Box::leak(
        resource
            .attributes
            .iter()
            .map(|attr| attr.name)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    let action = Box::leak(Box::new(
        ash_core::ActionDef::create("compile").accept(accepted),
    ));
    ash_core::DynamicChangeset::for_create(&ctx, resource, action, fields.clone())
        .unwrap()
        .with_upsert(identity, updates)
        .commit(&ctx)
        .await
        .unwrap();
    captured.lock().unwrap().take().unwrap()
}
