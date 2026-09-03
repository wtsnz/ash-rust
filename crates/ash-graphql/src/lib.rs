pub mod builder;
pub mod dataloader;
pub mod error;
pub mod filter;
pub mod mutation;
pub mod object;
pub mod pagination;
pub mod query;
pub mod sort;
pub mod types;

pub use builder::AshGraphQLBuilder;
pub use dataloader::{AshBatchLoader, BelongsToKey, HasManyKey, ManyToManyKey};
pub use error::{register_user_error, UserError};
pub use filter::{
    parse_resource_filter, register_primitive_filter_inputs, register_resource_filter_inputs,
};
pub use mutation::{
    build_action_mutation, mutation_input_name, mutation_name, mutation_payload_name,
    register_action_input, register_action_payload, MutationPayload,
};
pub use object::{build_resource_object, collect_enums_for_resource};
pub use pagination::{
    build_resource_connection_query, register_page_info, register_resource_connection_types,
    resource_connection_field_name, resource_connection_type_name, resource_edge_type_name,
};
pub use query::{build_resource_queries, get_query_name, list_query_name};
pub use sort::{parse_resource_sort, register_resource_sort_inputs};
pub use types::{
    ash_value_to_graphql_value, ash_value_to_graphql_value_typed, attr_type_to_type_ref,
    enum_type_name, graphql_value_to_ash_value,
};

/// Main facade for `ash-graphql`.
pub struct AshGraphQL;

impl AshGraphQL {
    /// Creates a schema builder from an Ash [`DomainDef`](ash_core::DomainDef).
    pub fn builder(domain: &'static ash_core::DomainDef) -> AshGraphQLBuilder {
        AshGraphQLBuilder::from_domain(domain)
    }

    /// Creates a schema builder from a slice of [`ResourceDef`](ash_core::ResourceDef)s.
    pub fn from_resources(resources: &[&'static ash_core::ResourceDef]) -> AshGraphQLBuilder {
        AshGraphQLBuilder::from_resources(resources)
    }

    /// Creates an `async-graphql` [`DataLoader`](async_graphql::dataloader::DataLoader) backed by [`AshBatchLoader`].
    pub fn create_dataloader<D: ash_core::DataLayer + Clone + 'static>(
        ctx: ash_core::Context<D>,
        resources: &[&'static ash_core::ResourceDef],
    ) -> async_graphql::dataloader::DataLoader<AshBatchLoader<D>> {
        let loader = AshBatchLoader::new(ctx, resources);
        async_graphql::dataloader::DataLoader::new(loader, tokio::spawn)
    }
}
