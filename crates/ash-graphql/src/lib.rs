pub mod builder;
pub mod object;
pub mod types;

pub use builder::AshGraphQLBuilder;
pub use object::{build_resource_object, collect_enums_for_resource};
pub use types::{
    ash_value_to_graphql_value, attr_type_to_type_ref, enum_type_name, graphql_value_to_ash_value,
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
