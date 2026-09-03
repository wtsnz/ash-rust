use ash_core::{DomainDef, ResourceDef};
use ash_pubsub::PubSub;
use async_graphql::dynamic::*;

use crate::object::{build_resource_object, collect_enums_for_resource};

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

    /// Builds the dynamic GraphQL schema.
    pub fn finish(self) -> Result<Schema, SchemaError> {
        let mut query = Object::new("Query");
        query = query.field(Field::new(
            "schema_version",
            TypeRef::named_nn(TypeRef::STRING),
            |_ctx| FieldFuture::new(async move {
                Ok(Some(FieldValue::value(async_graphql::Value::from("ash-graphql-0.1.0"))))
            }),
        ));

        // Connect each resource to Query root
        for res in &self.resources {
            let field_name = format!("get{}", res.name);
            let res_name = res.name;
            query = query.field(Field::new(
                field_name,
                TypeRef::named(res_name),
                |_ctx| FieldFuture::new(async move { Ok(None::<FieldValue>) }),
            ));
        }

        let mut builder = Schema::build("Query", None, None);

        // Register JSON scalar for arbitrary map values
        builder = builder.register(Scalar::new("JSON"));

        // Register all resources and their enums
        for res in &self.resources {
            let obj = build_resource_object(res);
            builder = builder.register(obj);

            for e in collect_enums_for_resource(res) {
                builder = builder.register(e);
            }
        }

        builder.register(query).finish()
    }
}
