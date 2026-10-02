pub mod builder;
pub mod dataloader;
pub mod error;
pub mod filter;
pub mod mutation;
pub mod names;
pub mod object;
pub mod pagination;
mod preload;
mod redact;
pub mod query;
pub(crate) mod request;
pub mod sort;
pub mod subscription;
pub mod types;

#[cfg(feature = "axum")]
pub mod axum;
#[cfg(feature = "axum")]
mod ws;

pub use builder::AshGraphQLBuilder;
pub use dataloader::{AshBatchLoader, RelatedKey};
pub use error::{UserError, register_user_error};
pub use builder::{ROOT_MUTATION, ROOT_QUERY, ROOT_SUBSCRIPTION};
pub use filter::{field_filter_input_name, parse_resource_filter, register_resource_filter_inputs};
pub use mutation::{
    MutationPayload, build_action_mutation, mutation_input_name, mutation_name,
    mutation_payload_name, register_action_input, register_action_payload,
};
pub use object::{build_resource_object, collect_enums_for_resource};
pub use pagination::{KeysetPage, MAX_PAGE_SIZE, build_keyset_query, keyset_page_type_name, register_keyset_page};
pub use query::{
    build_read_action_query, build_resource_queries, get_query_name, list_query_name,
    list_query_name_for_action,
};
pub use sort::{parse_resource_sort, register_resource_sort_inputs};
pub use subscription::{build_resource_subscriptions, subscription_result_name};
pub use types::{
    ash_value_to_graphql_value, ash_value_to_graphql_value_typed, attr_type_to_type_ref,
    enum_type_name, graphql_type_name, graphql_value_to_ash_value,
};

/// Main facade for `ash-graphql`.
pub struct AshGraphQL;

impl AshGraphQL {
    /// Creates a schema builder from an Ash [`DomainDef`](ash_core::DomainDef).
    pub fn builder(domain: &'static ash_core::DomainDef) -> AshGraphQLBuilder {
        AshGraphQLBuilder::from_domain(domain)
    }

    /// Creates a schema builder from an Ash [`DomainDef`](ash_core::DomainDef).
    pub fn from_domain(domain: &'static ash_core::DomainDef) -> AshGraphQLBuilder {
        AshGraphQLBuilder::from_domain(domain)
    }

    /// Creates a schema builder from a slice of [`ResourceDef`](ash_core::ResourceDef)s.
    pub fn from_resources(resources: &[&'static ash_core::ResourceDef]) -> AshGraphQLBuilder {
        AshGraphQLBuilder::from_resources(resources)
    }

    /// Creates an `async-graphql` [`DataLoader`](async_graphql::dataloader::DataLoader) backed
    /// by [`AshBatchLoader`], loading relationships as `ctx` reads them. Relationship
    /// resolvers only use it for a request running as the same actor and tenant.
    pub fn create_dataloader<D: ash_core::DataLayer + Clone + 'static>(
        ctx: ash_core::Context<D>,
    ) -> async_graphql::dataloader::DataLoader<AshBatchLoader<D>> {
        let loader = AshBatchLoader::new(ctx);
        async_graphql::dataloader::DataLoader::new(loader, tokio::spawn)
    }
}
