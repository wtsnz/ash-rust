//! Isomorphic TypeScript client SDK generator for Ash resources.

use ash_core::{ActionDef, ActionKind, AttrType, DomainDef, ResourceDef};
use crate::types::{input_fields, sort_fields, to_camel_case, to_pascal_case, to_upper_snake};

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

/// Getter query name (e.g. "Ticket" -> "getTicket").
pub fn get_query_name(name: &str) -> String {
    format!("get{}", name)
}

/// Mutation name for an action on a resource (e.g. "open" + "Ticket" -> "openTicket").
pub fn mutation_name(action: &ActionDef, res: &ResourceDef) -> String {
    format!("{}{}", to_camel_case(action.name), res.name)
}

/// Generate the isomorphic transport and base client runtime classes. With `live`, the
/// client config also takes the subscription connection's settings.
pub fn generate_transport_runtime(default_endpoint: &str, live: bool) -> String {
    let live_config = if live {
        r#"
  /** WebSocket endpoint for subscriptions, relative to `baseUrl` (defaults to `/graphql/ws`). */
  subscriptionEndpoint?: string;
  /** Full WebSocket URL for subscriptions, overriding `baseUrl` and `subscriptionEndpoint`. */
  subscriptionUrl?: string;
  /** WebSocket implementation, where there is no global `WebSocket`. */
  webSocket?: typeof WebSocket;
  /** Sent as the `connection_init` payload when the subscription connection opens. */
  connectionParams?:
    | Record<string, unknown>
    | (() => Record<string, unknown> | Promise<Record<string, unknown>>);"#
    } else {
        ""
    };
    format!(
        r#"// Ash Client Runtime & Transport
/** The most records a page holds: Ash's default `max_page_size`. */
export const ASH_PAGE_SIZE = 250;

export interface AshClientConfig {{
  baseUrl: string;
  graphqlEndpoint?: string;
  fetch?: typeof fetch;
  headers?: HeadersInit | (() => HeadersInit | Promise<HeadersInit>);{live_config}
}}

/** An error a mutation reports, as AshGraphql's `MutationError`, or a GraphQL error. */
export interface AshUserError {{
  message: string;
  shortMessage?: string | null;
  /** Ash's error code, e.g. `invalid_attribute`, `required`, `not_found`, `forbidden`. */
  code?: string | null;
  /** The input fields it's about. */
  fields?: string[];
  path?: (string | number)[];
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
      errors?: Array<{{
        message: string;
        path?: (string | number)[];
        code?: string;
        extensions?: {{ code?: string }};
      }}>;
    }};

    // A field a policy hides comes back null with a `forbidden_field` error: the null
    // stands for it, so it fails nothing.
    const errors = (json.errors ?? []).filter(
      (e) => (e.extensions?.code ?? e.code) !== "forbidden_field",
    );
    if (errors.length > 0) {{
      const userErrors: AshUserError[] = errors.map((e) => ({{
        message: e.message,
        path: e.path,
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

/// What a field of type `ty` selects of its value: an embedded resource's or typed map's
/// fields, or each union member's `value`, as GraphQL serves them; nothing for a scalar.
pub(crate) fn selection_of(ty: &AttrType) -> String {
    match ty {
        AttrType::Array { of } => selection_of(of),
        AttrType::Embedded(_) | AttrType::TypedMap { .. } => {
            let fields: Vec<String> =
                ty.fields().unwrap_or_default().iter().map(|field| format!("{}{}", to_camel_case(field.name), selection_of(&field.ty))).collect();
            format!(" {{ {} }}", fields.join(" "))
        }
        AttrType::Union { name, members } => {
            let members: Vec<String> = members
                .iter()
                .map(|member| format!("... on {name}{} {{ value{} }}", crate::types::to_pascal_case(member.name), selection_of(&member.ty)))
                .collect();
            format!(" {{ __typename {} }}", members.join(" "))
        }
        _ => String::new(),
    }
}

/// Generate selection set builder for a resource.
pub fn generate_selection_set_builder(res: &ResourceDef) -> String {
    let mut out = String::new();
    let name = res.name;

    let base_attrs = res
        .attributes
        .iter()
        .map(|a| format!("{}{}", to_camel_case(a.name), selection_of(&a.ty)))
        .collect::<Vec<_>>()
        .join(" ");

    out.push_str(&format!("export function build{name}SelectionSet(include?: {name}Include): string {{\n"));
    out.push_str(&format!("  let fields = \"{base_attrs}\";\n"));

    for rel in res.relationships {
        let rel_name = to_camel_case(rel.name);
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

/// The sort fields' names in the schema's `<Resource>SortField` enum, by the camelCase
/// name the client takes (e.g. `{ callSign: "CALL_SIGN" }`).
fn sort_field_names(res: &ResourceDef) -> String {
    let fields: Vec<String> = sort_fields(res)
        .into_iter()
        .map(|field| format!("{}: \"{}\"", to_camel_case(field), to_upper_snake(field)))
        .collect();
    format!("{{ {} }}", fields.join(", "))
}

/// How a resource's list query pages, as AshGraphql serves it for its primary read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Paging {
    Keyset,
    Offset,
    /// A plain list of every record.
    None,
}

impl Paging {
    /// As AshGraphql chooses for a list query: keyset where the read pages by keyset,
    /// else offset, else none.
    pub(crate) fn of(res: &ResourceDef) -> Self {
        match res.default_read().pagination {
            None => Self::None,
            Some(p) if p.keyset || !p.offset => Self::Keyset,
            Some(_) => Self::Offset,
        }
    }
}

/// Generate resource query builder class (e.g. `TicketQueryBuilder`). With `live`, it
/// also has `live()`.
pub fn generate_resource_query_builder(res: &ResourceDef, live: bool) -> String {
    let (constructor_params, _) = client_constructor(live);
    let live_method = if live {
        crate::live::generate_query_builder_live(res)
    } else {
        String::new()
    };
    query_builder_class(res.name, &sort_field_names(res), constructor_params, &live_method, Paging::of(res))
}

/// The reads of a query builder over a keyset-paged `list<Resource>s`.
fn keyset_reads(name: &str, list_q: &str) -> String {
    format!(
        r#"  private async read(
    paging: {{ first?: number; after?: string; last?: number; before?: string }},
    count: boolean,
  ): Promise<PaginatedResult<{name}>> {{
    const fields = build{name}SelectionSet(this._include);
    const query = `query List{name}($filter: {name}FilterInput, $sort: [{name}SortInput], $first: Int, $after: String, $last: Int, $before: String) {{
      {list_q}(filter: $filter, sort: $sort, first: $first, after: $after, last: $last, before: $before) {{
        results {{
          ${{fields}}
        }}
        startKeyset
        endKeyset${{count ? "\n        count" : ""}}
      }}
    }}`;

    const data = await this.transport.request<{{ {list_q}: PaginatedResult<{name}> }}>(query, {{
      filter: this._filter,
      sort: this.sortInput(),
      ...paging,
    }});
    return data.{list_q};
  }}

  /**
   * Every matching record, or the first `limit` of them. The read pages, so this reads
   * page after page, each following the last's end keyset.
   */
  public async all(): Promise<{name}[]> {{
    const records: {name}[] = [];
    let after: string | undefined;
    for (;;) {{
      const wanted = this._limit === undefined ? ASH_PAGE_SIZE : Math.min(ASH_PAGE_SIZE, this._limit - records.length);
      if (wanted <= 0) return records;
      const page = await this.read({{ first: wanted, after }}, false);
      records.push(...page.results);
      if (page.results.length < wanted || !page.endKeyset) return records;
      after = page.endKeyset;
    }}
  }}

  public async first(): Promise<{name} | null> {{
    const page = await this.read({{ first: 1 }}, false);
    return page.results[0] ?? null;
  }}

  /**
   * A keyset page: `first` records after the `after` keyset, or, given `before`, the
   * `first` records before it. Each page says the keysets at its ends, and how many
   * records match.
   */
  public async page(first: number = 20, after?: string, before?: string): Promise<PaginatedResult<{name}>> {{
    return before !== undefined
      ? this.read({{ last: first, before }}, true)
      : this.read({{ first, after }}, true);
  }}
"#
    )
}

/// The reads of a query builder over an offset-paged `list<Resource>s`.
fn offset_reads(name: &str, list_q: &str) -> String {
    format!(
        r#"  private async read(limit: number, offset: number, count: boolean): Promise<OffsetPage<{name}>> {{
    const fields = build{name}SelectionSet(this._include);
    const query = `query List{name}($filter: {name}FilterInput, $sort: [{name}SortInput], $limit: Int, $offset: Int) {{
      {list_q}(filter: $filter, sort: $sort, limit: $limit, offset: $offset) {{
        results {{
          ${{fields}}
        }}${{count ? "\n        count" : ""}}
      }}
    }}`;

    const data = await this.transport.request<{{ {list_q}: OffsetPage<{name}> }}>(query, {{
      filter: this._filter,
      sort: this.sortInput(),
      limit,
      offset,
    }});
    return data.{list_q};
  }}

  /**
   * Every matching record, or the first `limit` of them. The read pages, so this reads
   * page after page, each from where the last ended.
   */
  public async all(): Promise<{name}[]> {{
    const records: {name}[] = [];
    for (;;) {{
      const wanted = this._limit === undefined ? ASH_PAGE_SIZE : Math.min(ASH_PAGE_SIZE, this._limit - records.length);
      if (wanted <= 0) return records;
      const page = await this.read(wanted, records.length, false);
      records.push(...page.results);
      if (page.results.length < wanted) return records;
    }}
  }}

  public async first(): Promise<{name} | null> {{
    const page = await this.read(1, 0, false);
    return page.results[0] ?? null;
  }}

  /** An offset page: `limit` records after the first `offset`, and how many match. */
  public async page(limit: number = 20, offset: number = 0): Promise<OffsetPage<{name}>> {{
    return this.read(limit, offset, true);
  }}
"#
    )
}

/// The reads of a query builder over an unpaged `list<Resource>s`.
fn list_reads(name: &str, list_q: &str) -> String {
    format!(
        r#"  private async read(): Promise<{name}[]> {{
    const fields = build{name}SelectionSet(this._include);
    const query = `query List{name}($filter: {name}FilterInput, $sort: [{name}SortInput]) {{
      {list_q}(filter: $filter, sort: $sort) {{
        ${{fields}}
      }}
    }}`;

    const data = await this.transport.request<{{ {list_q}: {name}[] }}>(query, {{
      filter: this._filter,
      sort: this.sortInput(),
    }});
    return data.{list_q};
  }}

  /** Every matching record, or the first `limit` of them. The read doesn't page. */
  public async all(): Promise<{name}[]> {{
    const records = await this.read();
    return this._limit === undefined ? records : records.slice(0, this._limit);
  }}

  public async first(): Promise<{name} | null> {{
    return (await this.read())[0] ?? null;
  }}
"#
    )
}

/// A query builder class over the `list<Resource>s` read, paged as `paging` says.
/// `sort_names` maps each sort field the client takes to the schema's enum value for it.
pub(crate) fn query_builder_class(
    name: &str,
    sort_names: &str,
    constructor_params: &str,
    live_method: &str,
    paging: Paging,
) -> String {
    let list_q = list_query_name(name);
    let reads = match paging {
        Paging::Keyset => keyset_reads(name, &list_q),
        Paging::Offset => offset_reads(name, &list_q),
        Paging::None => list_reads(name, &list_q),
    };
    let page_query_options = match paging {
        Paging::Keyset => format!(
            r#"
  public pageQueryOptions(first: number = 20, after?: string, before?: string) {{
    return {{
      queryKey: [
        "{name}",
        "page",
        {{
          filter: this._filter,
          sort: this._sort,
          first,
          after,
          before,
          include: this._include,
        }},
      ],
      queryFn: () => this.page(first, after, before),
    }};
  }}
"#
        ),
        Paging::Offset => format!(
            r#"
  public pageQueryOptions(limit: number = 20, offset: number = 0) {{
    return {{
      queryKey: [
        "{name}",
        "page",
        {{
          filter: this._filter,
          sort: this._sort,
          limit,
          offset,
          include: this._include,
        }},
      ],
      queryFn: () => this.page(limit, offset),
    }};
  }}
"#
        ),
        Paging::None => String::new(),
    };
    format!(
        r#"const {name}SortFieldNames: Record<{name}SortField, string> = {sort_names};

export class {name}QueryBuilder {{
  private _filter?: {name}FilterInput;
  private _sort: {name}SortInput[] = [];
  private _limit?: number;
  private _include?: {name}Include;

  constructor({constructor_params}) {{}}

  public filter(filter?: {name}FilterInput): this {{
    this._filter = filter;
    return this;
  }}

  public sort(field: {name}SortField, order: SortOrder = "asc"): this {{
    this._sort.push({{ field, order }});
    return this;
  }}

  /** At most this many records, the first in the query's order. */
  public limit(limit: number): this {{
    this._limit = limit;
    return this;
  }}

  public include(include: {name}Include): this {{
    this._include = {{ ...this._include, ...include }};
    return this;
  }}

  /** The sort as the schema takes it: `[{{ field: "CALL_SIGN", order: "DESC" }}]`. */
  private sortInput() {{
    if (this._sort.length === 0) return undefined;
    return this._sort.map(({{ field, order }}) => ({{
      field: {name}SortFieldNames[field],
      order: order === "desc" ? "DESC" : "ASC",
    }}));
  }}

{reads}
  public queryOptions() {{
    return {{
      queryKey: [
        "{name}",
        "query",
        {{
          filter: this._filter,
          sort: this._sort,
          limit: this._limit,
          include: this._include,
        }},
      ],
      queryFn: () => this.all(),
    }};
  }}
{page_query_options}{live_method}}}

"#
    )
}

/// Generate individual resource client (e.g. `TicketClient`). With `live`, it also has
/// `onCreated`, `onUpdated` and `onDestroyed`.
pub fn generate_resource_client(res: &ResourceDef, live: bool) -> String {
    let mut out = String::new();
    let name = res.name;
    let get_q = get_query_name(name);
    let (constructor_params, builder_args) = client_constructor(live);

    out.push_str(&format!(
        r#"export class {name}Client {{
  constructor({constructor_params}) {{}}

  public query(): {name}QueryBuilder {{
    return new {name}QueryBuilder({builder_args});
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

    for action in res.actions {
        if matches!(action.kind, ActionKind::Create | ActionKind::Update | ActionKind::Destroy) {
            out.push_str(&generate_action_method(res, action));
        }
    }

    if live {
        out.push_str(&crate::live::generate_resource_subscription_methods(res));
    }
    out.push_str("}\n\n");
    out
}

/// A client method for a create, update or destroy, calling its mutation as ash-graphql
/// serves it: `<action><Resource>(id: ID!, input: <Action><Resource>Input)`, with no `id`
/// for a create and no `input` for an action that takes none. Every method takes an
/// input, optional unless the action requires some of it, so each kind of action is
/// called alike. A failure the mutation reports in `errors` throws an `AshClientError`
/// holding them.
fn generate_action_method(res: &ResourceDef, action: &ActionDef) -> String {
    let name = res.name;
    let m_name = mutation_name(action, res);
    let method = to_camel_case(action.name);
    let input_type = format!("{}{name}Input", to_pascal_case(action.name));
    let fields = input_fields(res, action);
    let has_id = action.kind != ActionKind::Create;
    let destroy = action.kind == ActionKind::Destroy;

    let mut params = Vec::new();
    let mut variables = Vec::new();
    let mut arguments = Vec::new();
    let mut values = Vec::new();
    if has_id {
        params.push("id: string".to_string());
        variables.push("$id: ID!".to_string());
        arguments.push("id: $id".to_string());
        values.push("id".to_string());
    }
    let required = fields.iter().any(|(_, _, required)| *required);
    params.push(if required {
        format!("input: {input_type}")
    } else if fields.is_empty() {
        // Nothing to send; taken so this method is called like the others.
        format!("_input: {input_type} = {{}}")
    } else {
        format!("input: {input_type} = {{}}")
    });
    if !fields.is_empty() {
        variables.push(format!("$input: {input_type}{}", if required { "!" } else { "" }));
        arguments.push("input: $input".to_string());
        values.push("input".to_string());
    }
    if !destroy {
        params.push(format!("include?: {name}Include"));
    }
    let params = params.join(", ");
    let variables = if variables.is_empty() {
        String::new()
    } else {
        format!("({})", variables.join(", "))
    };
    let arguments = if arguments.is_empty() {
        String::new()
    } else {
        format!("({})", arguments.join(", "))
    };
    let values = values.join(", ");

    let (returns, result_selection, check, value) = if destroy {
        ("boolean", "", "payload.errors.length > 0", "true")
    } else {
        (
            name,
            "\n        result {\n          ${fields}\n        }",
            "payload.errors.length > 0 || !payload.result",
            "payload.result",
        )
    };
    let fields_line = if destroy {
        ""
    } else {
        "\n    const fields = build{name}SelectionSet(include);"
    }
    .replace("{name}", name);

    format!(
        r#"
  public async {method}({params}): Promise<{returns}> {{{fields_line}
    const query = `mutation Mutate{name}{variables} {{
      {m_name}{arguments} {{{result_selection}
        errors {{
          message
          shortMessage
          code
          fields
        }}
      }}
    }}`;

    const data = await this.transport.request<{{
      {m_name}: {{
        result?: {name} | null;
        errors: AshUserError[];
      }};
    }}>(query, {{ {values} }});

    const payload = data.{m_name};
    if ({check}) {{
      throw new AshClientError(payload.errors[0]?.message || "Mutation failed", payload.errors);
    }}

    return {value};
  }}
"#
    )
}

/// Constructor parameters for a resource client or query builder, and the arguments that
/// pass them on.
fn client_constructor(live: bool) -> (&'static str, &'static str) {
    if live {
        (
            "private readonly transport: AshTransport, private readonly subscriptions: AshSubscriptionClient",
            "this.transport, this.subscriptions",
        )
    } else {
        ("private readonly transport: AshTransport", "this.transport")
    }
}

/// Generate domain client if multiple resources are grouped into a domain.
pub fn generate_domain_client(domain: &DomainDef, live: bool) -> String {
    let domain_pascal = to_pascal_case(domain.name);

    let mut out = String::new();
    out.push_str(&format!("export class {domain_pascal}DomainClient {{\n"));

    for res in domain.resources {
        let res_prop = to_camel_case(res.name);
        let res_client = format!("{}Client", res.name);
        out.push_str(&format!("  public readonly {res_prop}: {res_client};\n"));
    }

    let (params, args) = if live {
        ("transport: AshTransport, subscriptions: AshSubscriptionClient", "transport, subscriptions")
    } else {
        ("transport: AshTransport", "transport")
    };
    out.push_str(&format!("\n  constructor({params}) {{\n"));
    for res in domain.resources {
        let res_prop = to_camel_case(res.name);
        let res_client = format!("{}Client", res.name);
        out.push_str(&format!("    this.{res_prop} = new {res_client}({args});\n"));
    }
    out.push_str("  }\n}\n\n");

    out
}

/// Generate the root client class (e.g. `AshClient`).
pub fn generate_root_client(
    client_name: &str,
    resources: &[&'static ResourceDef],
    domains: &[&DomainDef],
    live: bool,
) -> String {
    let mut out = String::new();
    let args = if live { "this.transport, this.subscriptions" } else { "this.transport" };

    out.push_str(&format!("export class {client_name} {{\n"));
    out.push_str("  public readonly transport: AshTransport;\n");
    if live {
        out.push_str("  /** The live connection every subscription and live query shares. */\n");
        out.push_str("  public readonly subscriptions: AshSubscriptionClient;\n");
    }

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
    if live {
        out.push_str("    this.subscriptions = new AshSubscriptionClient(config);\n");
    }

    for res in resources {
        let prop = to_camel_case(res.name);
        let client_ty = format!("{}Client", res.name);
        out.push_str(&format!("    this.{prop} = new {client_ty}({args});\n"));
    }

    for domain in domains {
        let prop = to_camel_case(domain.name);
        let client_ty = format!("{}DomainClient", to_pascal_case(domain.name));
        out.push_str(&format!("    this.{prop} = new {client_ty}({args});\n"));
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
