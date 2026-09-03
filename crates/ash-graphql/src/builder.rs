use ash_core::{DataLayer, DomainDef, ResourceDef};
use ash_pubsub::PubSub;
use async_graphql::dynamic::*;

use crate::filter::{register_primitive_filter_inputs, register_resource_filter_inputs};
use crate::object::{build_resource_object, collect_enums_for_resource};
use crate::pagination::{
    build_resource_connection_query, register_page_info, register_resource_connection_types,
};
use crate::query::build_resource_queries;
use crate::sort::register_resource_sort_inputs;

/// High-level builder for creating an `async-graphql` [`Schema`] from Ash domains and resources.
pub struct AshGraphQLBuilder {
    pub(crate) resources: Vec<&'static ResourceDef>,
    pub(crate) pubsub: Option<PubSub>,
    pub(crate) dataloader_enabled: bool,
}

impl AshGraphQLBuilder {
    /// Creates a new builder from an Ash [`DomainDef`].
    pub fn from_domain(domain: &'static DomainDef) -> Self {
        Self {
            resources: domain.resources.to_vec(),
            pubsub: None,
            dataloader_enabled: false,
        }
    }

    /// Creates a new builder from a list of [`ResourceDef`]s.
    pub fn from_resources(resources: &[&'static ResourceDef]) -> Self {
        Self {
            resources: resources.to_vec(),
            pubsub: None,
            dataloader_enabled: false,
        }
    }

    /// Attaches an `ash-pubsub` [`PubSub`] instance for live GraphQL subscriptions.
    pub fn with_pubsub(mut self, pubsub: PubSub) -> Self {
        self.pubsub = Some(pubsub);
        self
    }

    /// Enables the `DataLoader` for batching relationship resolution.
    pub fn with_dataloader(mut self) -> Self {
        self.dataloader_enabled = true;
        self
    }

    /// Builds the dynamic GraphQL schema for the specified data layer context type `D`.
    pub fn finish<D: DataLayer + Clone + 'static>(self) -> Result<Schema, SchemaError> {
        let mut query = Object::new("Query");
        query = query.field(Field::new(
            "schema_version",
            TypeRef::named_nn(TypeRef::STRING),
            |_ctx| {
                FieldFuture::new(async move {
                    Ok(Some(FieldValue::value(async_graphql::Value::from(
                        "ash-graphql-0.1.0",
                    ))))
                })
            },
        ));

        // Connect read queries and Relay connection queries for each resource
        for res in &self.resources {
            let (get_field, list_field) = build_resource_queries::<D>(res);
            let conn_field = build_resource_connection_query::<D>(res);
            query = query.field(get_field).field(list_field).field(conn_field);
        }

        let mut builder = Schema::build("Query", None, None);

        // Register JSON scalar for arbitrary map values
        builder = builder.register(Scalar::new("JSON"));

        // Register shared PageInfo
        builder = register_page_info(builder);

        // Register primitive filters
        builder = register_primitive_filter_inputs(builder);

        // Register all resources, connections, filters, sorts, and enums
        for res in &self.resources {
            let obj = build_resource_object(res);
            builder = builder.register(obj);

            builder = register_resource_connection_types(builder, res);
            builder = register_resource_filter_inputs(builder, res);
            builder = register_resource_sort_inputs(builder, res);

            for e in collect_enums_for_resource(res) {
                builder = builder.register(e);
            }
        }

        builder.register(query).finish()
    }
}
