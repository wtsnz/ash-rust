//! Configuration options for TypeScript code generation.

/// Target module system for generated TypeScript.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ModuleType {
    /// Modern ECMAScript Modules (`import` / `export`).
    #[default]
    Esm,
    /// CommonJS (`require` / `module.exports`).
    CommonJs,
}

/// Configuration options for the TypeScript generator.
#[derive(Clone, Debug)]
pub struct TypeScriptConfig {
    /// Whether to generate Zod validation schemas for action inputs.
    pub generate_zod: bool,
    /// Whether to generate the isomorphic fetch-based client SDK.
    pub generate_client: bool,
    /// Whether to generate React / TanStack Query hooks.
    pub generate_react: bool,
    /// The name of the root client class (e.g. `AshClient`).
    pub client_class_name: String,
    /// Relative or absolute GraphQL endpoint on the backend (defaults to `/graphql`).
    pub graphql_endpoint: String,
    /// Target module type (defaults to ESM).
    pub module_type: ModuleType,
}

impl Default for TypeScriptConfig {
    fn default() -> Self {
        Self {
            generate_zod: true,
            generate_client: true,
            generate_react: true,
            client_class_name: "AshClient".to_string(),
            graphql_endpoint: "/graphql".to_string(),
            module_type: ModuleType::Esm,
        }
    }
}

impl TypeScriptConfig {
    /// Create a new configuration with default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable Zod schema generation.
    pub fn with_zod(mut self, enabled: bool) -> Self {
        self.generate_zod = enabled;
        self
    }

    /// Enable or disable isomorphic client SDK generation.
    pub fn with_client(mut self, enabled: bool) -> Self {
        self.generate_client = enabled;
        self
    }

    /// Enable or disable React / TanStack Query hook generation.
    pub fn with_react(mut self, enabled: bool) -> Self {
        self.generate_react = enabled;
        self
    }

    /// Set the name of the root client class.
    pub fn with_client_name(mut self, name: impl Into<String>) -> Self {
        self.client_class_name = name.into();
        self
    }

    /// Set the GraphQL endpoint.
    pub fn with_graphql_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.graphql_endpoint = endpoint.into();
        self
    }
}
