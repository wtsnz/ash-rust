//! React and TanStack Query hook generators.

use ash_core::{ActionKind, ResourceDef};
use crate::types::{to_camel_case, to_pascal_case};

/// Generate React / TanStack Query hooks for a given resource.
pub fn generate_resource_react_hooks(res: &ResourceDef, client_name: &str) -> String {
    let mut out = String::new();
    let name = res.name;
    let prop = to_camel_case(name);

    // 1. Single record query hook
    out.push_str(&format!(
        r#"export function use{name}(
  client: {client_name},
  id: string,
  include?: {name}Include,
  options?: Record<string, unknown>
) {{
  return {{
    ...client.{prop}.getQueryOptions(id, include),
    ...options,
  }};
}}

"#
    ));

    // 2. Collection query hook helper
    out.push_str(&format!(
        r#"export function use{name}Query(
  client: {client_name},
  params?: {{
    filter?: {name}FilterInput;
    limit?: number;
    offset?: number;
    include?: {name}Include;
  }},
  options?: Record<string, unknown>
) {{
  const builder = client.{prop}.query();
  if (params?.filter) builder.filter(params.filter);
  if (params?.limit) builder.limit(params.limit);
  if (params?.offset) builder.offset(params.offset);
  if (params?.include) builder.include(params.include);

  return {{
    ...builder.queryOptions(),
    ...options,
  }};
}}

"#
    ));

    // 3. Action mutation helpers
    for action in res.actions {
        let action_camel = to_camel_case(action.name);
        let action_pascal = to_pascal_case(action.name);
        let input_type = format!("{action_pascal}{name}Input");

        match action.kind {
            ActionKind::Create => {
                out.push_str(&format!(
                    r#"export function use{action_pascal}{name}Mutation(
  client: {client_name},
  options?: Record<string, unknown>
) {{
  return {{
    mutationFn: (variables: {{ input: {input_type}; include?: {name}Include }}) =>
      client.{prop}.{action_camel}(variables.input, variables.include),
    ...options,
  }};
}}

"#
                ));
            }
            ActionKind::Update => {
                out.push_str(&format!(
                    r#"export function use{action_pascal}{name}Mutation(
  client: {client_name},
  options?: Record<string, unknown>
) {{
  return {{
    mutationFn: (variables: {{ id: string; input: {input_type}; include?: {name}Include }}) =>
      client.{prop}.{action_camel}(variables.id, variables.input, variables.include),
    ...options,
  }};
}}

"#
                ));
            }
            ActionKind::Destroy => {
                out.push_str(&format!(
                    r#"export function use{action_pascal}{name}Mutation(
  client: {client_name},
  options?: Record<string, unknown>
) {{
  return {{
    mutationFn: (id: string) => client.{prop}.{action_camel}(id),
    ...options,
  }};
}}

"#
                ));
            }
            _ => {}
        }
    }

    out
}
