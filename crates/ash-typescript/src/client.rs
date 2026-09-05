//! Isomorphic TypeScript client SDK generator for Ash resources.

use ash_core::{ActionDef, ActionKind, DomainDef, ResourceDef};
use crate::types::{to_camel_case, to_pascal_case};

/// Returns plural suffix helper matching ash-graphql list and connection conventions.
pub fn plural_suffix(name: &str) -> &'static str {
    if name.ends_with('s')
        || name.ends_with('x')
        || name.ends_with('z')
        || name.ends_with("ch")
        || name.ends_with("sh")
    {
        "es"
    } else {
        "s"
    }
}

/// Pluralized name for list query (e.g. "Ticket" -> "listTickets").
pub fn list_query_name(name: &str) -> String {
    format!("list{}{}", name, plural_suffix(name))
}

/// Pluralized name for connection query (e.g. "Ticket" -> "ticketsConnection").
pub fn connection_query_name(name: &str) -> String {
    let lower_first = to_camel_case(name);
    format!("{}{suffix}Connection", lower_first, suffix = plural_suffix(&lower_first))
}

/// Getter query name (e.g. "Ticket" -> "getTicket").
pub fn get_query_name(name: &str) -> String {
    format!("get{}", name)
}

/// Mutation name for an action on a resource (e.g. "open" + "Ticket" -> "openTicket").
pub fn mutation_name(action: &ActionDef, res: &ResourceDef) -> String {
    format!("{}{}", to_camel_case(action.name), res.name)
}

/// Generate the isomorphic transport and base client runtime classes.
pub fn generate_transport_runtime(default_endpoint: &str) -> String {
    format!(
        r#"// Ash Client Runtime & Transport
export interface AshClientConfig {{
  baseUrl: string;
  graphqlEndpoint?: string;
  fetch?: typeof fetch;
  headers?: HeadersInit | (() => HeadersInit | Promise<HeadersInit>);
}}

export interface AshUserError {{
  field?: string;
  message: string;
}}

export class AshClientError extends Error {{
  public readonly errors: AshUserError[];

  constructor(message: string, errors: AshUserError[] = []) {{
    super(message);
    this.name = "AshClientError";
    this.errors = errors;
  }}
}}

export class AshTransport {{
  private readonly defaultEndpoint = "{default_endpoint}";

  constructor(private readonly config: AshClientConfig) {{}}

  public async request<T>(query: string, variables: Record<string, unknown> = {{}}): Promise<T> {{
    const fetchFn = this.config.fetch ?? (typeof fetch !== "undefined" ? fetch : undefined);
    if (!fetchFn) {{
      throw new Error("No fetch implementation available. Please pass a custom fetch to AshClientConfig.");
    }}

    const base = this.config.baseUrl.replace(/\/$/, "");
    const endpoint = this.config.graphqlEndpoint ?? this.defaultEndpoint;
    const url = `${{base}}${{endpoint.startsWith("/") ? "" : "/"}}${{endpoint}}`;

    let customHeaders: HeadersInit = {{}};
    if (typeof this.config.headers === "function") {{
      customHeaders = await this.config.headers();
    }} else if (this.config.headers) {{
      customHeaders = this.config.headers;
    }}

    const res = await fetchFn(url, {{
      method: "POST",
      headers: {{
        "Content-Type": "application/json",
        "Accept": "application/json",
        ...customHeaders,
      }},
      body: JSON.stringify({{ query, variables }}),
    }});

    if (!res.ok) {{
      throw new AshClientError(`HTTP error ${{res.status}}: ${{res.statusText}}`);
    }}

    const json = (await res.json()) as {{
      data?: T;
      errors?: Array<{{ message: string; path?: string[] }}>;
    }};

    if (json.errors && json.errors.length > 0) {{
      const userErrors: AshUserError[] = json.errors.map((e) => ({{
        field: e.path ? e.path.join(".") : undefined,
        message: e.message,
      }}));
      throw new AshClientError(userErrors[0].message || "GraphQL execution error", userErrors);
    }}

    if (!json.data) {{
      throw new AshClientError("GraphQL returned empty data response");
    }}

    return json.data;
  }}
}}
"#
    )
}

/// Generate selection set builder for a resource.
pub fn generate_selection_set_builder(res: &ResourceDef) -> String {
    let mut out = String::new();
    let name = res.name;

    let base_attrs = res
        .attributes
        .iter()
        .map(|a| a.name)
        .collect::<Vec<_>>()
        .join(" ");

    out.push_str(&format!("export function build{name}SelectionSet(include?: {name}Include): string {{\n"));
    out.push_str(&format!("  let fields = \"{base_attrs}\";\n"));

    for rel in res.relationships {
        let rel_name = rel.name;
        let dest_name = (rel.destination)().name;
        out.push_str(&format!("  if (include?.{rel_name}) {{\n"));
        out.push_str(&format!("    const subInclude = typeof include.{rel_name} === \"object\" ? include.{rel_name} : undefined;\n"));
        out.push_str(&format!("    fields += ` {rel_name} {{ ${{build{dest_name}SelectionSet(subInclude)}} }}`;\n"));
        out.push_str("  }\n");
    }

    out.push_str("  return fields;\n");
    out.push_str("}\n\n");
    out
}

/// Generate resource query builder class (e.g. `TicketQueryBuilder`).
pub fn generate_resource_query_builder(res: &ResourceDef) -> String {
    let mut out = String::new();
    let name = res.name;
    let list_q = list_query_name(name);
    let conn_q = connection_query_name(name);

    out.push_str(&format!(
        r#"export class {name}QueryBuilder {{
  private _filter?: {name}FilterInput;
  private _sort: {name}SortInput[] = [];
  private _limit?: number;
  private _offset?: number;
  private _include?: {name}Include;

  constructor(private readonly transport: AshTransport) {{}}

  public filter(filter?: {name}FilterInput): this {{
    this._filter = filter;
    return this;
  }}

  public sort(field: {name}SortField, order: SortOrder = "asc"): this {{
    this._sort.push({{ field, order }});
    return this;
  }}

  public limit(limit: number): this {{
    this._limit = limit;
    return this;
  }}

  public offset(offset: number): this {{
    this._offset = offset;
    return this;
  }}

  public include(include: {name}Include): this {{
    this._include = {{ ...this._include, ...include }};
    return this;
  }}

  public async all(): Promise<{name}[]> {{
    const fields = build{name}SelectionSet(this._include);
    const query = `query List{name}($filter: {name}FilterInput, $sort: [{name}SortInput!], $limit: Int, $offset: Int) {{
      {list_q}(filter: $filter, sort: $sort, limit: $limit, offset: $offset) {{
        ${{fields}}
      }}
    }}`;

    const data = await this.transport.request<{{ {list_q}: {name}[] }}>(query, {{
      filter: this._filter,
      sort: this._sort.length > 0 ? this._sort : undefined,
      limit: this._limit,
      offset: this._offset,
    }});

    return data.{list_q};
  }}

  public async first(): Promise<{name} | null> {{
    this._limit = 1;
    const list = await this.all();
    return list[0] ?? null;
  }}

  public async page(first: number = 20, after?: string): Promise<PaginatedResult<{name}>> {{
    const fields = build{name}SelectionSet(this._include);
    const query = `query Conn{name}($filter: {name}FilterInput, $sort: [{name}SortInput!], $first: Int, $after: String) {{
      {conn_q}(filter: $filter, sort: $sort, first: $first, after: $after) {{
        edges {{
          node {{
            ${{fields}}
          }}
          cursor
        }}
        pageInfo {{
          hasNextPage
          hasPreviousPage
          startCursor
          endCursor
        }}
        totalCount
      }}
    }}`;

    const data = await this.transport.request<{{
      {conn_q}: {{
        edges: Array<{{ node: {name}; cursor: string }}>;
        pageInfo: PageInfo;
        totalCount?: number;
      }};
    }}>(query, {{
      filter: this._filter,
      sort: this._sort.length > 0 ? this._sort : undefined,
      first,
      after,
    }});

    const conn = data.{conn_q};
    return {{
      results: conn.edges.map((e) => e.node),
      pageInfo: conn.pageInfo,
      totalCount: conn.totalCount,
    }};
  }}

  public queryOptions() {{
    return {{
      queryKey: [
        "{name}",
        "query",
        {{
          filter: this._filter,
          sort: this._sort,
          limit: this._limit,
          offset: this._offset,
          include: this._include,
        }},
      ],
      queryFn: () => this.all(),
    }};
  }}

  public pageQueryOptions(first: number = 20, after?: string) {{
    return {{
      queryKey: [
        "{name}",
        "page",
        {{
          filter: this._filter,
          sort: this._sort,
          first,
          after,
          include: this._include,
        }},
      ],
      queryFn: () => this.page(first, after),
    }};
  }}
}}

"#
    ));

    out
}

/// Generate individual resource client (e.g. `TicketClient`).
pub fn generate_resource_client(res: &ResourceDef) -> String {
    let mut out = String::new();
    let name = res.name;
    let get_q = get_query_name(name);

    out.push_str(&format!(
        r#"export class {name}Client {{
  constructor(private readonly transport: AshTransport) {{}}

  public query(): {name}QueryBuilder {{
    return new {name}QueryBuilder(this.transport);
  }}

  public async get(id: string, include?: {name}Include): Promise<{name} | null> {{
    const fields = build{name}SelectionSet(include);
    const query = `query Get{name}($id: ID!) {{
      {get_q}(id: $id) {{
        ${{fields}}
      }}
    }}`;

    const data = await this.transport.request<{{ {get_q}: {name} | null }}>(query, {{ id }});
    return data.{get_q};
  }}

  public getQueryOptions(id: string, include?: {name}Include) {{
    return {{
      queryKey: ["{name}", "get", id, include],
      queryFn: () => this.get(id, include),
    }};
  }}
"#
    ));

    // Actions
    for action in res.actions {
        let m_name = mutation_name(action, res);
        let action_method_name = to_camel_case(action.name);
        let action_pascal = to_pascal_case(action.name);
        let input_type = format!("{name}{action_pascal}Input");

        match action.kind {
            ActionKind::Create => {
                out.push_str(&format!(
                    r#"
  public async {action_method_name}(input: {input_type}, include?: {name}Include): Promise<{name}> {{
    const fields = build{name}SelectionSet(include);
    const query = `mutation Mutate{name}($input: {input_type}!) {{
      {m_name}(input: $input) {{
        result {{
          ${{fields}}
        }}
        errors {{
          field
          message
        }}
        success
      }}
    }}`;

    const data = await this.transport.request<{{
      {m_name}: {{
        result?: {name};
        errors: AshUserError[];
        success: boolean;
      }};
    }}>(query, {{ input }});

    const payload = data.{m_name};
    if (!payload.success || !payload.result) {{
      throw new AshClientError(payload.errors[0]?.message || "Mutation failed", payload.errors);
    }}

    return payload.result;
  }}
"#
                ));
            }
            ActionKind::Update => {
                out.push_str(&format!(
                    r#"
  public async {action_method_name}(id: string, input: {input_type}, include?: {name}Include): Promise<{name}> {{
    const fields = build{name}SelectionSet(include);
    const query = `mutation Mutate{name}($id: ID!, $input: {input_type}!) {{
      {m_name}(id: $id, input: $input) {{
        result {{
          ${{fields}}
        }}
        errors {{
          field
          message
        }}
        success
      }}
    }}`;

    const data = await this.transport.request<{{
      {m_name}: {{
        result?: {name};
        errors: AshUserError[];
        success: boolean;
      }};
    }}>(query, {{ id, input }});

    const payload = data.{m_name};
    if (!payload.success || !payload.result) {{
      throw new AshClientError(payload.errors[0]?.message || "Mutation failed", payload.errors);
    }}

    return payload.result;
  }}
"#
                ));
            }
            ActionKind::Destroy => {
                out.push_str(&format!(
                    r#"
  public async {action_method_name}(id: string): Promise<boolean> {{
    const query = `mutation Mutate{name}($id: ID!) {{
      {m_name}(id: $id) {{
        errors {{
          field
          message
        }}
        success
      }}
    }}`;

    const data = await this.transport.request<{{
      {m_name}: {{
        errors: AshUserError[];
        success: boolean;
      }};
    }}>(query, {{ id }});

    const payload = data.{m_name};
    if (!payload.success) {{
      throw new AshClientError(payload.errors[0]?.message || "Destroy failed", payload.errors);
    }}

    return true;
  }}
"#
                ));
            }
            ActionKind::Read => {
                // Reads are handled by query() and get()
            }
            ActionKind::Generic => {
                // Generic action
                out.push_str(&format!(
                    r#"
  public async {action_method_name}(input?: Record<string, unknown>): Promise<unknown> {{
    const query = `mutation Mutate{name}($input: GenericInput) {{
      {m_name}(input: $input) {{
        result
        errors {{
          field
          message
        }}
        success
      }}
    }}`;

    const data = await this.transport.request<{{
      {m_name}: {{
        result?: unknown;
        errors: AshUserError[];
        success: boolean;
      }};
    }}>(query, {{ input: input ?? {{}} }});

    const payload = data.{m_name};
    if (!payload.success) {{
      throw new AshClientError(payload.errors[0]?.message || "Action failed", payload.errors);
    }}

    return payload.result;
  }}
"#
                ));
            }
        }
    }

    out.push_str("}\n\n");
    out
}

/// Generate domain client if multiple resources are grouped into a domain.
pub fn generate_domain_client(domain: &DomainDef) -> String {
    let domain_pascal = to_pascal_case(domain.name);

    let mut out = String::new();
    out.push_str(&format!("export class {domain_pascal}DomainClient {{\n"));

    for res in domain.resources {
        let res_prop = to_camel_case(res.name);
        let res_client = format!("{}Client", res.name);
        out.push_str(&format!("  public readonly {res_prop}: {res_client};\n"));
    }

    out.push_str("\n  constructor(transport: AshTransport) {\n");
    for res in domain.resources {
        let res_prop = to_camel_case(res.name);
        let res_client = format!("{}Client", res.name);
        out.push_str(&format!("    this.{res_prop} = new {res_client}(transport);\n"));
    }
    out.push_str("  }\n}\n\n");

    out
}

/// Generate the root client class (e.g. `AshClient`).
pub fn generate_root_client(
    client_name: &str,
    resources: &[&'static ResourceDef],
    domains: &[&DomainDef],
) -> String {
    let mut out = String::new();

    out.push_str(&format!("export class {client_name} {{\n"));
    out.push_str("  public readonly transport: AshTransport;\n");

    // Direct resource properties
    for res in resources {
        let prop = to_camel_case(res.name);
        let client_ty = format!("{}Client", res.name);
        out.push_str(&format!("  public readonly {prop}: {client_ty};\n"));
    }

    // Domain properties
    for domain in domains {
        let prop = to_camel_case(domain.name);
        let client_ty = format!("{}DomainClient", to_pascal_case(domain.name));
        out.push_str(&format!("  public readonly {prop}: {client_ty};\n"));
    }

    out.push_str("\n  constructor(config: AshClientConfig) {\n");
    out.push_str("    this.transport = new AshTransport(config);\n");

    for res in resources {
        let prop = to_camel_case(res.name);
        let client_ty = format!("{}Client", res.name);
        out.push_str(&format!("    this.{prop} = new {client_ty}(this.transport);\n"));
    }

    for domain in domains {
        let prop = to_camel_case(domain.name);
        let client_ty = format!("{}DomainClient", to_pascal_case(domain.name));
        out.push_str(&format!("    this.{prop} = new {client_ty}(this.transport);\n"));
    }

    out.push_str("  }\n}\n\n");

    // Factory helper
    let factory_name = format!("create{}", client_name);
    out.push_str(&format!(
        r#"export function {factory_name}(config: AshClientConfig): {client_name} {{
  return new {client_name}(config);
}}
"#
    ));

    out
}
