pub mod builder;
pub mod filter;
pub mod object;
pub mod query;
pub mod sort;
pub mod types;

pub use builder::AshGraphQLBuilder;
pub use filter::{parse_resource_filter, register_primitive_filter_inputs, register_resource_filter_inputs};
pub use object::{build_resource_object, collect_enums_for_resource};
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
}
