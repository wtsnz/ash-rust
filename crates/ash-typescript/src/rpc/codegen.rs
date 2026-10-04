//! AshTypescript's typed RPC client, generated from an [`Rpc`]: `ash_types.ts` (each
//! resource's schema, filter input and field lists, and the types that infer a result
//! from a selection) and `ash_rpc.ts` (a function per RPC action, with its input,
//! config and result types), as `mix ash_typescript.codegen` writes them, so a client
//! written against an AshTypescript app runs against an ash-rust one unchanged.
//!
//! The parts that don't depend on the app (the utility types, and the helpers the
//! functions send their requests through) are AshTypescript's own output, kept as it
//! writes them: © ash_typescript contributors, MIT licensed.

use ash_core::{
    ActionDef, ActionKind, AggregateKind, AttrType, Countable, MetadataDef, RelKind, ResourceDef,
};

use super::{Identity, Rpc, RpcAction};
use crate::types::{to_camel_case, to_pascal_case};

const UTILITY_TYPES: &str = include_str!("codegen/utility_types.ts");

/// How the client is generated, as AshTypescript's codegen options set it.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Where `ash_rpc.ts` imports `ash_types.ts` from.
    pub types_import: String,
    /// Where actions run (`run_endpoint`): `/rpc/run` by default.
    pub run_endpoint: Endpoint,
    /// Where inputs are validated (`validate_endpoint`): `/rpc/validate` by default.
    pub validate_endpoint: Endpoint,
    /// Whether each action gets a validation function (`generate_validation_functions`).
    pub validation_functions: bool,
    /// Whether each action gets Phoenix channel functions (`generate_phx_channel_rpc_actions`).
    pub channel_functions: bool,
    /// Where `Channel` is imported from (`phoenix_import_path`).
    pub phoenix_import: String,
    /// The client's lifecycle hooks.
    pub hooks: Hooks,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            types_import: "./ash_types".into(),
            run_endpoint: Endpoint::Path("/rpc/run".into()),
            validate_endpoint: Endpoint::Path("/rpc/validate".into()),
            validation_functions: false,
            channel_functions: false,
            phoenix_import: "phoenix".into(),
            hooks: Hooks::default(),
        }
    }
}

/// Where the client sends a request: a path, or a TypeScript expression evaluated at
/// runtime (AshTypescript's `{:runtime_expr, ...}`).
#[derive(Clone, Debug)]
pub enum Endpoint {
    Path(String),
    Expression(String),
}

impl Endpoint {
    fn ts(&self) -> String {
        match self {
            Self::Path(path) => format!("\"{path}\""),
            Self::Expression(expr) => expr.clone(),
        }
    }
}

/// The client's lifecycle hooks, as AshTypescript names them: TypeScript functions
/// called around each request or channel push, and the types of the context each
/// takes (`hookCtx`).
#[derive(Clone, Debug, Default)]
pub struct Hooks {
    pub action_before_request: Option<String>,
    pub action_after_request: Option<String>,
    pub validation_before_request: Option<String>,
    pub validation_after_request: Option<String>,
    pub action_before_channel_push: Option<String>,
    pub action_after_channel_response: Option<String>,
    pub validation_before_channel_push: Option<String>,
    pub validation_after_channel_response: Option<String>,
    pub action_context_type: Option<String>,
    pub validation_context_type: Option<String>,
    pub action_channel_context_type: Option<String>,
    pub validation_channel_context_type: Option<String>,
}

impl Hooks {
    fn action(&self) -> bool {
        self.action_before_request.is_some() || self.action_after_request.is_some()
    }
    fn validation(&self) -> bool {
        self.validation_before_request.is_some() || self.validation_after_request.is_some()
    }
    fn action_channel(&self) -> bool {
        self.action_before_channel_push.is_some() || self.action_after_channel_response.is_some()
    }
    fn validation_channel(&self) -> bool {
        self.validation_before_channel_push.is_some() || self.validation_after_channel_response.is_some()
    }
}

/// A typed query (AshTypescript's `typed_query`): a read with fields chosen up front,
/// generated as a result type and a fields constant.
#[derive(Clone, Debug)]
pub(crate) struct TypedQuery {
    pub name: String,
    pub resource: &'static ResourceDef,
    pub action: &'static ActionDef,
    pub fields: serde_json::Value,
    pub result_type_name: Option<String>,
    pub fields_const_name: Option<String>,
    pub description: Option<String>,
}

/// The generated client's two files.
#[derive(Clone, Debug)]
pub struct Client {
    /// `ash_types.ts`.
    pub types: String,
    /// `ash_rpc.ts`.
    pub rpc: String,
}

impl Client {
    /// Writes `ash_types.ts` and `ash_rpc.ts` into `dir`.
    pub fn write(&self, dir: impl AsRef<std::path::Path>) -> std::io::Result<()> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)?;
        std::fs::write(dir.join("ash_types.ts"), &self.types)?;
        std::fs::write(dir.join("ash_rpc.ts"), &self.rpc)
    }
}

impl<D> Rpc<D> {
    /// AshTypescript's client for these actions.
    pub fn typescript_client(&self, config: &ClientConfig) -> Client {
        let actions = self.ordered_actions();
        let mut resources: Vec<&'static ResourceDef> = Vec::new();
        for (_, rpc) in &actions {
            if !resources.iter().any(|resource| resource.name == rpc.resource.name) {
                resources.push(rpc.resource);
            }
        }
        resources.sort_by_key(|resource| resource.name);
        let types = types_file(&resources, &actions);
        let rpc = rpc_file(&actions, &self.typed_queries, &types, config);
        Client { types, rpc }
    }

    /// The actions, grouped by resource and ordered by the action each runs, as
    /// AshTypescript lists them; actions running the same one in the order declared.
    fn ordered_actions(&self) -> Vec<(&str, &RpcAction<D>)> {
        let mut actions: Vec<(usize, &str, &RpcAction<D>)> = self
            .declared
            .iter()
            .enumerate()
            .filter_map(|(i, name)| self.actions.get(name).map(|rpc| (i, name.as_str(), rpc)))
            .collect();
        actions.sort_by(|a, b| (a.2.resource.name, a.2.action.name, a.0).cmp(&(b.2.resource.name, b.2.action.name, b.0)));
        actions.into_iter().map(|(_, name, rpc)| (name, rpc)).collect()
    }
}

// ── Types ───────────────────────────────────────────────────────────────────

/// The TypeScript type of a value of type `ty`, as AshTypescript maps Ash's types.
fn ts_type(ty: &AttrType) -> String {
    match ty {
        AttrType::Uuid => "UUID".into(),
        AttrType::String | AttrType::CiString | AttrType::Inet => "string".into(),
        AttrType::Integer | AttrType::Float => "number".into(),
        AttrType::Decimal => "Decimal".into(),
        AttrType::Boolean => "boolean".into(),
        AttrType::Date => "AshDate".into(),
        AttrType::Binary => "Binary".into(),
        AttrType::UtcDatetime { precision: ash_core::TimePrecision::Second } => "UtcDateTime".into(),
        AttrType::UtcDatetime { .. } => "UtcDateTimeUsec".into(),
        AttrType::Vector { .. } => "Array<number>".into(),
        AttrType::Atom { one_of, .. } if !one_of.is_empty() => {
            let mut values: Vec<&str> = one_of.to_vec();
            values.sort_unstable();
            values.iter().map(|value| format!("\"{value}\"")).collect::<Vec<_>>().join(" | ")
        }
        AttrType::Atom { .. } => "string".into(),
        AttrType::Map | AttrType::Embedded(_) | AttrType::TypedMap { .. } | AttrType::Union { .. } => "Record<string, any>".into(),
        AttrType::Array { of } => format!("Array<{}>", ts_type(of)),
    }
}

/// The aliases a type needs, as AshTypescript declares them.
fn alias_of(ty: &AttrType) -> Option<&'static str> {
    match ty {
        AttrType::Uuid => Some("export type UUID = string;"),
        AttrType::Decimal => Some("export type Decimal = string;"),
        AttrType::Date => Some("export type AshDate = string;"),
        AttrType::Binary => Some("export type Binary = string;"),
        AttrType::UtcDatetime { precision: ash_core::TimePrecision::Second } => Some("export type UtcDateTime = string;"),
        AttrType::UtcDatetime { .. } => Some("export type UtcDateTimeUsec = string;"),
        AttrType::Array { of } => alias_of(of),
        _ => None,
    }
}

/// A field a client selects, filters or sorts by: an attribute, calculation or aggregate.
struct Field {
    name: &'static str,
    ty: AttrType,
    allow_nil: bool,
    kind: FieldKind,
}

enum FieldKind {
    Attribute,
    Calculation(&'static [ash_core::ArgumentDef]),
    Aggregate,
}

/// Every field of `resource`, in name order, as AshTypescript's manifest lists them.
fn fields(resource: &ResourceDef) -> Vec<Field> {
    let mut fields: Vec<Field> = resource
        .attributes
        .iter()
        .map(|attr| Field { name: attr.name, ty: attr.ty, allow_nil: attr.allow_nil, kind: FieldKind::Attribute })
        .chain(resource.calculations.iter().map(|calc| Field {
            name: calc.name,
            ty: calc.ty,
            allow_nil: true,
            kind: FieldKind::Calculation(calc.arguments),
        }))
        .chain(resource.aggregates.iter().map(|agg| Field {
            name: agg.name,
            ty: match agg.kind {
                AggregateKind::Exists => AttrType::Boolean,
                AggregateKind::Count => AttrType::Integer,
                _ => agg.ty,
            },
            allow_nil: !matches!(agg.kind, AggregateKind::Count | AggregateKind::Exists),
            kind: FieldKind::Aggregate,
        }))
        .collect();
    fields.sort_by_key(|field| field.name);
    fields
}

/// The relationships of `resource` to resources the client has, in name order.
fn relationships<'a>(resource: &'a ResourceDef, resources: &[&'static ResourceDef]) -> Vec<&'a ash_core::RelationshipDef> {
    let mut rels: Vec<_> = resource
        .relationships
        .iter()
        .filter(|rel| resources.iter().any(|known| known.name == (rel.destination)().name))
        .collect();
    rels.sort_by_key(|rel| rel.name);
    rels
}

fn to_many(rel: &ash_core::RelationshipDef) -> bool {
    matches!(rel.kind, RelKind::HasMany | RelKind::ManyToMany)
}

/// Whether a to-one relationship may find nothing.
fn rel_allow_nil(resource: &ResourceDef, rel: &ash_core::RelationshipDef) -> bool {
    match rel.kind {
        RelKind::BelongsTo => rel.source_columns().iter().any(|column| resource.attribute(column).is_none_or(|attr| attr.allow_nil)),
        _ => true,
    }
}

/// How the relationship's destination read pages, as AshTypescript marks it.
fn rel_pagination(rel: &ash_core::RelationshipDef) -> Option<&'static str> {
    let pagination = (rel.destination)().primary_read()?.pagination?;
    match (pagination.offset, pagination.keyset) {
        (true, true) => Some("mixed"),
        (true, false) => Some("offset"),
        (false, true) => Some("keyset"),
        (false, false) => None,
    }
}

fn calc_args_type(args: &[ash_core::ArgumentDef]) -> String {
    if args.is_empty() {
        return "{}".into();
    }
    let args: Vec<String> = args
        .iter()
        .map(|arg| {
            let base = ts_type(&arg.ty);
            let ty = if arg.allow_nil { format!("{base} | null") } else { base };
            let optional = arg.default.is_some() || arg.allow_nil;
            format!("{}{}: {ty}", to_camel_case(arg.name), if optional { "?" } else { "" })
        })
        .collect();
    format!("{{ {} }}", args.join("; "))
}

fn primitive_union(names: &[&str]) -> String {
    if names.is_empty() {
        return "never".into();
    }
    names.iter().map(|name| format!("\"{}\"", to_camel_case(name))).collect::<Vec<_>>().join(" | ")
}

fn field_line(field: &Field) -> String {
    let ty = ts_type(&field.ty);
    if field.allow_nil {
        format!("  {}: {ty} | null;", to_camel_case(field.name))
    } else {
        format!("  {}: {ty};", to_camel_case(field.name))
    }
}

fn resource_schema(resource: &ResourceDef, resources: &[&'static ResourceDef]) -> String {
    let fields = fields(resource);
    let (complex, primitive): (Vec<&Field>, Vec<&Field>) =
        fields.iter().partition(|field| matches!(field.kind, FieldKind::Calculation(args) if !args.is_empty()));
    let mut lines = vec![
        "  __type: \"Resource\";".to_string(),
        format!("  __primitiveFields: {};", primitive_union(&primitive.iter().map(|field| field.name).collect::<Vec<_>>())),
    ];
    lines.extend(primitive.iter().map(|field| field_line(field)));
    for field in complex {
        let FieldKind::Calculation(args) = field.kind else { continue };
        let base = ts_type(&field.ty);
        let returns = if field.allow_nil { format!("{base} | null") } else { base };
        lines.push(format!(
            "  {}: {{ __type: \"ComplexCalculation\"; __returnType: {returns}; __args: {}; }};",
            to_camel_case(field.name),
            calc_args_type(args)
        ));
    }
    for rel in relationships(resource, resources) {
        let dest = (rel.destination)().name;
        let line = if to_many(rel) {
            let pagination = rel_pagination(rel).map(|kind| format!(" __pagination: \"{kind}\";")).unwrap_or_default();
            format!(
                "{{ __type: \"Relationship\"; __array: true; __resource: {dest}ResourceSchema;{pagination} __filterInput: {dest}FilterInput; __sortField: {dest}SortField; }}"
            )
        } else {
            let null = if rel_allow_nil(resource, rel) { " | null" } else { "" };
            format!("{{ __type: \"Relationship\"; __resource: {dest}ResourceSchema{null}; }}")
        };
        lines.push(format!("  {}: {line};", to_camel_case(rel.name)));
    }
    format!("export type {}ResourceSchema = {{\n{}\n}};\n", resource.name, lines.join("\n"))
}

fn attributes_only_schema(resource: &ResourceDef) -> String {
    let attrs: Vec<Field> = fields(resource).into_iter().filter(|field| matches!(field.kind, FieldKind::Attribute)).collect();
    let mut lines = vec![
        "  __type: \"Resource\";".to_string(),
        format!("  __primitiveFields: {};", primitive_union(&attrs.iter().map(|field| field.name).collect::<Vec<_>>())),
    ];
    lines.extend(attrs.iter().map(field_line));
    format!("export type {}AttributesOnlySchema = {{\n{}\n}};\n", resource.name, lines.join("\n"))
}

/// The operators a field of type `ty` filters by, as Ash offers them for the type.
fn operators(ty: &AttrType) -> &'static [&'static str] {
    const COMPARE: &[&str] = &["eq", "notEq", "in", "lessThan", "greaterThan", "lessThanOrEqual", "greaterThanOrEqual"];
    const TEXT: &[&str] = &[
        "eq", "notEq", "in", "lessThan", "greaterThan", "lessThanOrEqual", "greaterThanOrEqual", "contains", "stringEndsWith",
        "stringStartsWith",
    ];
    match ty {
        AttrType::Boolean => &["eq", "notEq", "in"],
        AttrType::String | AttrType::CiString => TEXT,
        AttrType::Map | AttrType::Embedded(_) | AttrType::TypedMap { .. } | AttrType::Union { .. } => &["eq", "notEq", "in"],
        AttrType::Array { .. } => &["eq", "notEq", "in", "has"],
        _ => COMPARE,
    }
}

fn filter_input(resource: &ResourceDef, resources: &[&'static ResourceDef]) -> String {
    let name = format!("{}FilterInput", resource.name);
    let field_filters: Vec<String> = fields(resource)
        .iter()
        .map(|field| {
            let base = ts_type(&field.ty);
            let mut lines = Vec::new();
            if field.allow_nil {
                lines.push("    isNil?: boolean;".to_string());
            }
            for op in operators(&field.ty) {
                let rhs = match *op {
                    "in" => format!("Array<{base}>"),
                    "has" => match &field.ty {
                        AttrType::Array { of } => ts_type(of),
                        _ => base.clone(),
                    },
                    _ => base.clone(),
                };
                lines.push(format!("    {op}?: {rhs};"));
            }
            format!("  {}?: {{\n{}\n  }};\n", to_camel_case(field.name), lines.join("\n"))
        })
        .collect();
    let rel_filters: Vec<String> = relationships(resource, resources)
        .iter()
        .map(|rel| format!("  {}?: {}FilterInput;\n", to_camel_case(rel.name), (rel.destination)().name))
        .collect();
    format!(
        "export type {name} = {{\n  and?: Array<{name}>;\n  or?: Array<{name}>;\n  not?: Array<{name}>;\n\n{}\n{}\n}};\n",
        field_filters.join("\n"),
        rel_filters.join("\n")
    )
}

fn lower_first(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map(|first| first.to_ascii_lowercase().to_string() + chars.as_str()).unwrap_or_default()
}

fn field_array(resource: &ResourceDef, suffix: &str, names: &[String]) -> String {
    if names.is_empty() {
        return String::new();
    }
    let items = names.iter().map(|name| format!("\"{name}\"")).collect::<Vec<_>>().join(", ");
    let const_name = format!("{}{suffix}s", lower_first(resource.name));
    format!(
        "export const {const_name} = [{items}] as const;\nexport type {}{suffix} = (typeof {const_name})[number];\n",
        resource.name
    )
}

fn filter_field_array(resource: &ResourceDef, resources: &[&'static ResourceDef]) -> String {
    let mut names: Vec<String> = fields(resource).iter().map(|field| to_camel_case(field.name)).collect();
    names.extend(relationships(resource, resources).iter().map(|rel| to_camel_case(rel.name)));
    field_array(resource, "FilterField", &names)
}

fn sort_field_array(resource: &ResourceDef) -> String {
    let names: Vec<String> = fields(resource).iter().map(|field| to_camel_case(field.name)).collect();
    field_array(resource, "SortField", &names)
}

fn type_aliases<D>(resources: &[&'static ResourceDef], actions: &[(&str, &RpcAction<D>)]) -> String {
    let mut types: Vec<AttrType> = Vec::new();
    for resource in resources {
        types.extend(fields(resource).iter().map(|field| field.ty));
        types.extend(resource.calculations.iter().flat_map(|calc| calc.arguments.iter().map(|arg| arg.ty)));
        types.extend(resource.actions.iter().flat_map(|action| action.arguments.iter().map(|arg| arg.ty)));
    }
    for (_, rpc) in actions {
        types.extend(rpc.action.returns);
        types.extend(rpc.action.metadata.iter().map(|def| def.ty));
    }
    let mut aliases: Vec<&str> = types.iter().filter_map(alias_of).collect();
    aliases.sort_unstable();
    aliases.dedup();
    aliases.join("\n")
}

fn types_file<D>(resources: &[&'static ResourceDef], actions: &[(&str, &RpcAction<D>)]) -> String {
    let schemas: Vec<String> = resources
        .iter()
        .map(|resource| {
            let base = format!("// {} Schema\n{}\n", resource.name, resource_schema(resource, resources));
            [base, attributes_only_schema(resource)].join("\n\n")
        })
        .collect();
    let filters: String = resources.iter().map(|resource| filter_input(resource, resources)).collect();
    let filter_arrays: Vec<String> = resources.iter().map(|resource| filter_field_array(resource, resources)).collect();
    let sort_arrays: Vec<String> = resources.iter().map(|resource| sort_field_array(resource)).collect();
    format!(
        "// Generated by AshTypescript - Shared Types\n// Do not edit this file manually\n\n\n\n{}\n\n{}\n\n{}\n\n{}\n\n{}\n\n{}",
        type_aliases(resources, actions),
        schemas.join("\n\n"),
        filters,
        filter_arrays.join("\n"),
        sort_arrays.join("\n"),
        UTILITY_TYPES
    )
}

// ── RPC functions ──────────────────────────────────────────────────────────

/// What an action's function takes and does, as AshTypescript's action context has it.
struct Context {
    identities: Vec<String>,
    supports_pagination: bool,
    supports_filtering: bool,
    supports_sorting: bool,
    input: InputKind,
    is_get: bool,
}

#[derive(PartialEq)]
enum InputKind {
    None,
    Required,
    Optional,
}

/// One input of an action: an accepted attribute or an argument.
struct Input {
    name: &'static str,
    ty: AttrType,
    allow_nil: bool,
    required: bool,
}

fn inputs(resource: &ResourceDef, action: &ActionDef) -> Vec<Input> {
    let mut inputs = Vec::new();
    if matches!(action.kind, ActionKind::Create | ActionKind::Update) {
        for name in action.accept {
            if let Some(attr) = resource.attribute(name) {
                let has_default = attr.default_fn.is_some() || attr.generated;
                inputs.push(Input {
                    name: attr.name,
                    ty: attr.ty,
                    allow_nil: attr.allow_nil,
                    required: action.kind == ActionKind::Create && !attr.allow_nil && !has_default,
                });
            }
        }
    }
    for arg in action.arguments {
        inputs.push(Input { name: arg.name, ty: arg.ty, allow_nil: arg.allow_nil, required: !arg.allow_nil && arg.default.is_none() });
    }
    inputs
}

fn context<D>(rpc: &RpcAction<D>) -> Context {
    let action = rpc.action;
    let options = &rpc.options;
    let is_get = action.kind == ActionKind::Read && (options.get || !options.get_by.is_empty());
    let list = action.kind == ActionKind::Read && !is_get;
    let inputs = inputs(rpc.resource, action);
    Context {
        identities: if matches!(action.kind, ActionKind::Update | ActionKind::Destroy) {
            options.identities.iter().map(identity_type(rpc.resource, false)).collect()
        } else {
            Vec::new()
        },
        supports_pagination: list && action.pagination.is_some_and(|p| p.keyset || p.offset),
        supports_filtering: list && options.enable_filter,
        supports_sorting: list && options.enable_sort,
        input: if inputs.is_empty() {
            InputKind::None
        } else if inputs.iter().any(|input| input.required) {
            InputKind::Required
        } else {
            InputKind::Optional
        },
        is_get,
    }
}

/// How an identity is given; a validation function takes text for any of its values.
fn identity_type(resource: &ResourceDef, validation: bool) -> impl Fn(&Identity) -> String + '_ {
    let ts_type = move |ty: &AttrType| {
        let ty = ts_type(ty);
        if validation && ty != "string" { format!("{ty} | string") } else { ty }
    };
    move |identity| match identity {
        Identity::PrimaryKey => {
            let keys: Vec<_> = resource.attributes.iter().filter(|attr| attr.primary_key).collect();
            match keys.as_slice() {
                [key] => ts_type(&key.ty),
                keys => format!(
                    "{{ {} }}",
                    keys.iter().map(|key| format!("{}: {}", to_camel_case(key.name), ts_type(&key.ty))).collect::<Vec<_>>().join("; ")
                ),
            }
        }
        Identity::Named(name) => match resource.identities.iter().find(|i| i.name == *name) {
            Some(identity) => format!(
                "{{ {} }}",
                identity
                    .keys
                    .iter()
                    .map(|key| {
                        let ty = resource.attribute(key).map_or_else(|| "string".to_string(), |attr| ts_type(&attr.ty));
                        format!("{}: {ty}", to_camel_case(key))
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            None => format!("{{ /* Identity {name} not found */ never }}"),
        },
    }
}

/// The metadata an action's function exposes.
fn exposed_metadata<D>(rpc: &RpcAction<D>) -> Vec<&'static MetadataDef> {
    rpc.action
        .metadata
        .iter()
        .filter(|def| rpc.options.show_metadata.as_ref().is_none_or(|shown| shown.contains(&def.name)))
        .collect()
}

fn metadata_name<D>(rpc: &RpcAction<D>, def: &MetadataDef) -> String {
    let mapped = rpc.options.metadata_field_names.iter().find(|(field, _)| *field == def.name).map_or(def.name, |(_, name)| name);
    to_camel_case(mapped)
}

fn input_type<D>(pascal: &str, rpc: &RpcAction<D>) -> String {
    let inputs = inputs(rpc.resource, rpc.action);
    if inputs.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = inputs
        .iter()
        .map(|input| {
            let base = ts_type(&input.ty);
            let ty = if input.allow_nil { format!("{base} | null") } else { base };
            format!("  {}{}: {ty};", to_camel_case(input.name), if input.required { "" } else { "?" })
        })
        .collect();
    format!("export type {pascal}Input = {{\n{}\n}};\n", lines.join("\n"))
}

fn metadata_type<D>(pascal: &str, rpc: &RpcAction<D>) -> String {
    let exposed = exposed_metadata(rpc);
    if exposed.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = exposed
        .iter()
        .map(|def| format!("  {}{}: {};", metadata_name(rpc, def), if def.allow_nil { "?" } else { "" }, ts_type(&def.ty)))
        .collect();
    format!("\nexport type {pascal}Metadata = {{\n{}\n}};\n", lines.join("\n"))
}

fn keyset_inline(schema: &str, array: &str) -> String {
    let _ = schema;
    format!(
        "{{\n  results: {array};\n  hasMore: boolean;\n  limit: number;\n  after: string | null;\n  before: string | null;\n  previousPage: string | null;\n  nextPage: string | null;\n  count?: number | null;\n  type: \"keyset\";\n}}"
    )
}

fn offset_inline(array: &str) -> String {
    format!("{{\n  results: {array};\n  hasMore: boolean;\n  limit: number;\n  offset: number;\n  count?: number | null;\n  type: \"offset\";\n}}")
}

fn result_type<D>(pascal: &str, rpc: &RpcAction<D>, ctx: &Context) -> String {
    let schema = format!("{}ResourceSchema", rpc.resource.name);
    let has_metadata = !exposed_metadata(rpc).is_empty();
    let selection = match (rpc.options.enable_filter, rpc.options.enable_sort) {
        (true, true) => format!("UnifiedFieldSelection<{schema}>"),
        (f, s) => format!("UnifiedFieldSelection<{schema}, {f}, {s}>"),
    };
    let metadata = metadata_type(pascal, rpc);
    let metadata_param = format!("MetadataFields extends ReadonlyArray<keyof {pascal}Metadata> = []");
    let picked = format!("Pick<{pascal}Metadata, MetadataFields[number]>");
    match rpc.action.kind {
        ActionKind::Read if ctx.is_get => {
            let null = if rpc.options.not_found_error.unwrap_or(true) { "" } else { " | null" };
            if has_metadata {
                format!(
                    "export type {pascal}Fields = {selection}[];\n{metadata}\nexport type Infer{pascal}Result<\n  Fields extends {pascal}Fields,\n  {metadata_param}\n> = (InferResult<{schema}, Fields> & {picked}){null};\n"
                )
            } else {
                format!("export type {pascal}Fields = {selection}[];\nexport type Infer{pascal}Result<\n  Fields extends {pascal}Fields,\n> = InferResult<{schema}, Fields>{null};\n")
            }
        }
        ActionKind::Read if ctx.supports_pagination => {
            let pagination = rpc.action.pagination.expect("paged");
            let fields_type = format!("export type {pascal}Fields = {selection}[];\n");
            let array = if has_metadata {
                format!("Array<InferResult<{schema}, Fields> & {picked}>")
            } else {
                format!("Array<InferResult<{schema}, Fields>>")
            };
            let metadata_line = if has_metadata { format!("  {metadata_param},\n") } else { String::new() };
            let page = if pagination.required {
                // Pages required: the page alone.
                let body = match (pagination.offset, pagination.keyset) {
                    (true, true) => format!("{} | {}", offset_inline(&array), keyset_inline(&schema, &array)),
                    (true, false) => offset_inline(&array),
                    _ => keyset_inline(&schema, &array),
                };
                let metadata_line = if has_metadata { format!("  {metadata_param}\n") } else { String::new() };
                let fields_line = format!("  Fields extends {pascal}Fields,\n");
                format!("export type Infer{pascal}Result<\n{fields_line}{metadata_line}> = {body};\n")
            } else {
                let wrapped = match (pagination.offset, pagination.keyset) {
                    (true, true) => format!(
                        "ConditionalPaginatedResultMixed<Page, {array}, {}, {}>",
                        offset_inline(&array),
                        keyset_inline(&schema, &array)
                    ),
                    (true, false) => format!("ConditionalPaginatedResult<Page, {array}, {}>", offset_inline(&array)),
                    _ => format!("ConditionalPaginatedResult<Page, {array}, {}>", keyset_inline(&schema, &array)),
                };
                format!(
                    "export type Infer{pascal}Result<\n  Fields extends {pascal}Fields | undefined,\n{metadata_line}  Page extends {pascal}Config[\"page\"] = undefined\n> = {wrapped};\n"
                )
            };
            format!("{fields_type}{metadata}\n\n{page}")
        }
        ActionKind::Read => {
            if has_metadata {
                format!(
                    "export type {pascal}Fields = {selection}[];\n{metadata}\nexport type Infer{pascal}Result<\n  Fields extends {pascal}Fields,\n  {metadata_param}\n> = Array<InferResult<{schema}, Fields> & {picked}>;\n"
                )
            } else {
                format!("export type {pascal}Fields = {selection}[];\nexport type Infer{pascal}Result<\n  Fields extends {pascal}Fields,\n> = Array<InferResult<{schema}, Fields>>;\n")
            }
        }
        ActionKind::Create | ActionKind::Update => {
            if has_metadata {
                format!(
                    "export type {pascal}Fields = {selection}[];\n{metadata}\nexport type Infer{pascal}Result<\n  Fields extends {pascal}Fields | undefined,\n  {metadata_param}\n> = InferResult<{schema}, Fields>;\n"
                )
            } else {
                format!("export type {pascal}Fields = {selection}[];\n\nexport type Infer{pascal}Result<\n  Fields extends {pascal}Fields | undefined,\n> = InferResult<{schema}, Fields>;\n")
            }
        }
        ActionKind::Destroy => {
            if has_metadata {
                format!("{metadata}\nexport type Infer{pascal}Result<\n  {metadata_param}\n> = {{}};\n")
            } else {
                metadata
            }
        }
        ActionKind::Generic => {
            let returns = match rpc.action.returns {
                Some(AttrType::Map) => "Record<string, any>".to_string(),
                Some(AttrType::Array { of: AttrType::Map }) => "Array<Record<string, any>>".to_string(),
                Some(ty) => ts_type(&ty),
                None => "{}".to_string(),
            };
            format!("export type Infer{pascal}Result = {returns};\n")
        }
    }
}

const FETCH_CONFIG: [&str; 3] = [
    "  headers?: Record<string, string>;",
    "  fetchOptions?: RequestInit;",
    "  customFetch?: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;",
];

fn pagination_config(pagination: ash_core::Pagination) -> Vec<String> {
    let optional = if pagination.required { "" } else { "?" };
    let limit = if pagination.required && pagination.default_limit.is_none() { "" } else { "?" };
    let countable = pagination.countable != Countable::No;
    match (pagination.offset, pagination.keyset) {
        (true, true) => {
            let mut lines = vec![format!("  page{optional}: ("), "    {".into(), format!("      limit{limit}: number;"), "      offset?: number;".into()];
            if countable {
                lines.push("      count?: boolean;".into());
            }
            lines.extend(["    } | {".into(), format!("      limit{limit}: number;"), "      after?: string;".into(), "      before?: string;".into(), "    }".into(), "  );".into()]);
            lines
        }
        (true, false) => {
            let mut lines = vec![
                format!("  page{optional}: {{"),
                format!("    limit{limit}: number;"),
                "    offset?: number;".into(),
                "    after?: never;".into(),
                "    before?: never;".into(),
            ];
            if countable {
                lines.push("    count?: boolean;".into());
            }
            lines.push("  };".into());
            lines
        }
        _ => vec![
            format!("  page{optional}: {{"),
            format!("    limit{limit}: number;"),
            "    after?: string;".into(),
            "    before?: string;".into(),
            "    offset?: never;".into(),
            "    count?: never;".into(),
            "  };".into(),
        ],
    }
}

fn description(resource: &ResourceDef, action: &ActionDef) -> (&'static str, String) {
    match action.kind {
        ActionKind::Read => ("read", format!("Read {} records", resource.name)),
        ActionKind::Create => ("create", format!("Create a new {}", resource.name)),
        ActionKind::Update => ("update", format!("Update an existing {}", resource.name)),
        ActionKind::Destroy => ("destroy", format!("Delete a {}", resource.name)),
        ActionKind::Generic => ("action", format!("Execute generic action on {}", resource.name)),
    }
}

fn jsdoc(resource: &ResourceDef, action: &ActionDef) -> String {
    let (kind, description) = description(resource, action);
    format!("/**\n * {description}\n *\n * @ashActionType :{kind}\n */")
}

fn validation_jsdoc(resource: &ResourceDef, action: &ActionDef) -> String {
    let (kind, description) = description(resource, action);
    format!("/**\n * Validate: {description}\n *\n * @ashActionType :{kind}\n * @validation true\n */")
}

/// Which request a config is for, and so which hook context it carries.
#[derive(Clone, Copy, PartialEq)]
enum Request {
    Action,
    Validation,
    ActionChannel,
    ValidationChannel,
}

/// The config every function takes: tenant, identity, input, and the hook context.
fn common_config<D>(pascal: &str, rpc: &RpcAction<D>, ctx: &Context, request: Request, hooks: &Hooks) -> Vec<String> {
    let validation = matches!(request, Request::Validation | Request::ValidationChannel);
    let mut config = vec!["  tenant?: string;".to_string()];
    if !ctx.identities.is_empty() {
        let identities: Vec<String> = rpc
            .options
            .identities
            .iter()
            .map(|identity| identity_type(rpc.resource, validation)(identity))
            .collect();
        config.push(format!("  identity: {};", identities.join(" | ")));
    }
    match ctx.input {
        InputKind::Required => config.push(format!("  input: {pascal}Input;")),
        InputKind::Optional => config.push(format!("  input?: {pascal}Input;")),
        InputKind::None => {}
    }
    let hook = match request {
        Request::ValidationChannel if hooks.validation_channel() => Some("ValidationChannelHookContext"),
        Request::ActionChannel if hooks.action_channel() => Some("ActionChannelHookContext"),
        Request::Validation if hooks.validation() => Some("ValidationHookContext"),
        Request::Action if hooks.action() => Some("ActionHookContext"),
        _ => None,
    };
    if let Some(hook) = hook {
        config.push(format!("  hookCtx?: {hook};"));
    }
    config
}

/// What an execution function takes and answers, whatever it's sent over.
struct Shape {
    config: Vec<String>,
    has_fields: bool,
    fields_generic: String,
    has_metadata: bool,
    optional_pagination: bool,
    is_mutation: bool,
}

fn shape<D>(pascal: &str, rpc: &RpcAction<D>, ctx: &Context, request: Request, hooks: &Hooks) -> Shape {
    let action = rpc.action;
    let resource = rpc.resource;
    let has_metadata = !exposed_metadata(rpc).is_empty();
    let mut config = common_config(pascal, rpc, ctx, request, hooks);
    if !rpc.options.get_by.is_empty() {
        config.push("  getBy: {".into());
        for field in &rpc.options.get_by {
            let ty = resource.attribute(field).map_or_else(|| "string".to_string(), |attr| ts_type(&attr.ty));
            config.push(format!("    {}: {ty};", to_camel_case(field)));
        }
        config.push("  };".into());
    }
    let (has_fields, fields_generic) = match action.kind {
        ActionKind::Read => {
            config.push("  fields: Fields;".into());
            (true, format!("Fields extends {pascal}Fields"))
        }
        ActionKind::Create | ActionKind::Update => {
            config.push("  fields?: Fields;".into());
            (true, format!("Fields extends {pascal}Fields | undefined = undefined"))
        }
        _ => (false, String::new()),
    };
    if ctx.supports_filtering {
        config.push(format!("  filter?: {}FilterInput;", resource.name));
    }
    if ctx.supports_sorting {
        config.push(format!("  sort?: SortString<{0}SortField> | SortString<{0}SortField>[];", resource.name));
    }
    if ctx.supports_pagination {
        config.extend(pagination_config(action.pagination.expect("paged")));
    }
    if has_metadata {
        config.push("  metadataFields?: MetadataFields;".into());
    }
    let optional_pagination =
        action.kind == ActionKind::Read && !ctx.is_get && ctx.supports_pagination && !action.pagination.is_some_and(|p| p.required) && has_fields;
    Shape {
        config,
        has_fields,
        fields_generic,
        has_metadata,
        optional_pagination,
        is_mutation: matches!(action.kind, ActionKind::Create | ActionKind::Update),
    }
}

/// The payload a function sends.
fn payload(name: &str, rpc: &RpcAction<impl Sized>, ctx: &Context, fields: bool, metadata: bool, filtering: bool, get_by: bool) -> String {
    let mut payload = vec![format!("action: \"{name}\""), "...(config.tenant !== undefined && { tenant: config.tenant })".to_string()];
    if !ctx.identities.is_empty() {
        payload.push("identity: config.identity".into());
    }
    if get_by && !rpc.options.get_by.is_empty() {
        payload.push("getBy: config.getBy".into());
    }
    if ctx.input != InputKind::None {
        payload.push("input: config.input".into());
    }
    if fields {
        payload.push("...(config.fields !== undefined && { fields: config.fields })".into());
    }
    if metadata {
        payload.push("...(config.metadataFields && { metadataFields: config.metadataFields })".into());
    }
    if filtering && ctx.supports_filtering {
        payload.push("...(config.filter && { filter: config.filter })".into());
    }
    if filtering && ctx.supports_sorting {
        payload.push("...(config.sort && { sort: Array.isArray(config.sort) ? config.sort.join(\",\") : config.sort })".into());
    }
    if filtering && ctx.supports_pagination {
        payload.push("...(config.page && { page: config.page })".into());
    }
    format!("{{\n    {}\n  }}", payload.join(",\n    "))
}

fn execution_function<D>(name: &str, pascal: &str, rpc: &RpcAction<D>, ctx: &Context, hooks: &Hooks) -> String {
    let action = rpc.action;
    let shape = shape(pascal, rpc, ctx, Request::Action, hooks);
    let mut config = shape.config.clone();
    config.extend(FETCH_CONFIG.iter().map(|line| line.to_string()));
    let (has_fields, has_metadata, optional_pagination, is_mutation) =
        (shape.has_fields, shape.has_metadata, shape.optional_pagination, shape.is_mutation);
    let fields_generic = shape.fields_generic.clone();

    let (config_export, config_ref) = if optional_pagination {
        let concrete: Vec<String> = config
            .iter()
            .map(|line| {
                line.replace(": Fields;", &format!(": {pascal}Fields;"))
                    .replace(": MetadataFields;", &format!(": ReadonlyArray<keyof {pascal}Metadata>;"))
            })
            .collect();
        (format!("export type {pascal}Config = {{\n{}\n}};\n\n", concrete.join("\n")), format!("{pascal}Config"))
    } else {
        (String::new(), format!("{{\n{}\n}}", config.join("\n")))
    };

    // The result type, and the function's generics and signature.
    let error = "{ success: false; errors: AshRpcError[]; }\n";
    let metadata_param = format!("MetadataFields extends ReadonlyArray<keyof {pascal}Metadata> = []");
    let picked = format!("Pick<{pascal}Metadata, MetadataFields[number]>");
    let (result_def, returns, generics, signature) = if action.kind == ActionKind::Destroy {
        if has_metadata {
            (
                format!("export type {pascal}Result<{metadata_param}> = | {{ success: true; data: {{}}; metadata: {picked}; }}\n| {error}\n;"),
                format!("{pascal}Result<MetadataFields>"),
                metadata_param.clone(),
                format!("config: {config_ref}"),
            )
        } else {
            (format!("export type {pascal}Result = | {{ success: true; data: {{}}; }}\n| {error}\n;"), format!("{pascal}Result"), String::new(), format!("config: {config_ref}"))
        }
    } else if has_fields {
        let mutation_metadata = if is_mutation && has_metadata { format!(" metadata: {picked};") } else { String::new() };
        let config_generic = format!("Config extends {pascal}Config = {pascal}Config");
        let (result_generics, data_generics, fn_generics, sig, fn_return) = if optional_pagination && has_metadata {
            (
                format!("{fields_generic}, {metadata_param}, Page extends {pascal}Config[\"page\"] = undefined"),
                "<Fields, MetadataFields, Page>".to_string(),
                format!("{fields_generic}, {config_generic}"),
                "config: Config & { fields: Fields }".to_string(),
                "<Fields, Config[\"metadataFields\"] extends ReadonlyArray<any> ? Config[\"metadataFields\"] : [], Config[\"page\"]>".to_string(),
            )
        } else if optional_pagination {
            (
                format!("{fields_generic}, Page extends {pascal}Config[\"page\"] = undefined"),
                "<Fields, Page>".to_string(),
                format!("{fields_generic}, {config_generic}"),
                "config: Config & { fields: Fields }".to_string(),
                "<Fields, Config[\"page\"]>".to_string(),
            )
        } else if action.kind == ActionKind::Read && has_metadata {
            (
                format!("{fields_generic}, {metadata_param}"),
                "<Fields, MetadataFields>".to_string(),
                format!("{fields_generic}, {metadata_param}"),
                format!("config: {config_ref}"),
                "<Fields, MetadataFields>".to_string(),
            )
        } else if is_mutation && has_metadata {
            (
                format!("{fields_generic}, {metadata_param}"),
                "<Fields>".to_string(),
                format!("{fields_generic}, {metadata_param}"),
                format!("config: {config_ref}"),
                "<Fields extends undefined ? [] : Fields, MetadataFields>".to_string(),
            )
        } else if action.kind == ActionKind::Read {
            (fields_generic.clone(), "<Fields>".to_string(), fields_generic.clone(), format!("config: {config_ref}"), "<Fields>".to_string())
        } else {
            (
                fields_generic.clone(),
                "<Fields>".to_string(),
                fields_generic.clone(),
                format!("config: {config_ref}"),
                "<Fields extends undefined ? [] : Fields>".to_string(),
            )
        };
        (
            format!("export type {pascal}Result<{result_generics}> = | {{ success: true; data: Infer{pascal}Result{data_generics};{mutation_metadata} }}\n| {error}\n;"),
            format!("{pascal}Result{fn_return}"),
            fn_generics,
            sig,
        )
    } else if has_metadata {
        (
            format!("export type {pascal}Result<{metadata_param}> = | {{ success: true; data: Infer{pascal}Result; metadata: {picked}; }}\n| {error}\n;"),
            format!("{pascal}Result<MetadataFields>"),
            metadata_param.clone(),
            format!("config: {config_ref}"),
        )
    } else {
        (format!("export type {pascal}Result = | {{ success: true; data: Infer{pascal}Result; }}\n| {error}\n;"), format!("{pascal}Result"), String::new(), format!("config: {config_ref}"))
    };

    let payload = payload(name, rpc, ctx, has_fields, has_metadata, true, true);
    let generics = if generics.is_empty() { String::new() } else { format!("<{generics}>") };
    format!(
        "{config_export}{result_def}\n\n{}\nexport async function {}{generics}(\n  {signature}\n): Promise<{returns}> {{\n  const payload = {payload};\n\n  return executeActionRpcRequest<{returns}>(\n    payload,\n    config\n  );\n}}\n",
        jsdoc(rpc.resource, action),
        to_camel_case(name),
    )
}

fn validation_function<D>(name: &str, pascal: &str, rpc: &RpcAction<D>, ctx: &Context, hooks: &Hooks) -> String {
    let mut config = common_config(pascal, rpc, ctx, Request::Validation, hooks);
    config.extend(FETCH_CONFIG.iter().map(|line| line.to_string()));
    let payload = payload(name, rpc, ctx, false, false, false, false);
    format!(
        "{}\nexport async function {}(\n  config: {{\n{}\n}}\n): Promise<ValidationResult> {{\n  const payload = {payload};\n\n  return executeValidationRpcRequest<ValidationResult>(\n    payload,\n    config\n  );\n}}\n",
        validation_jsdoc(rpc.resource, rpc.action),
        to_camel_case(&format!("validate_{name}")),
        config.join("\n"),
    )
}

/// `config`, its page field one `page?: Page;`, so a channel function's result narrows
/// to the page its caller asks for.
fn page_generic(config: &[String]) -> Vec<String> {
    let Some(start) = config.iter().position(|line| line.starts_with("  page?:")) else {
        return config.to_vec();
    };
    let mut out = config[..start].to_vec();
    out.push("  page?: Page;".into());
    let rest = &config[start + 1..];
    let rest = if config[start].ends_with(';') {
        rest
    } else {
        let close = rest.iter().position(|line| line == "  );" || line == "  };").map_or(rest.len(), |i| i + 1);
        &rest[close..]
    };
    out.extend(rest.iter().cloned());
    out
}

fn channel_function<D>(name: &str, pascal: &str, rpc: &RpcAction<D>, ctx: &Context, hooks: &Hooks) -> String {
    let shape = shape(pascal, rpc, ctx, Request::ActionChannel, hooks);
    let destroy = rpc.action.kind == ActionKind::Destroy;
    let page_param = shape.optional_pagination.then(|| format!("Page extends {pascal}Config[\"page\"] = undefined"));
    let config = if page_param.is_some() { page_generic(&shape.config) } else { shape.config.clone() };
    let metadata_param = format!("MetadataFields extends ReadonlyArray<keyof {pascal}Metadata> = []");
    let (mut params, mut args): (Vec<String>, Vec<&str>) = if destroy || !shape.has_fields {
        if shape.has_metadata { (vec![metadata_param], vec!["MetadataFields"]) } else { (vec![], vec![]) }
    } else if shape.has_metadata {
        (vec![shape.fields_generic.clone(), metadata_param], vec!["Fields", "MetadataFields"])
    } else {
        (vec![shape.fields_generic.clone()], vec!["Fields"])
    };
    if let Some(page) = &page_param
        && shape.has_fields
        && !destroy
    {
        params.push(page.clone());
        args.push("Page");
    }
    let result = if args.is_empty() { format!("{pascal}Result") } else { format!("{pascal}Result<{}>", args.join(", ")) };
    let generics = if params.is_empty() { String::new() } else { format!("<{}>", params.join(", ")) };
    let mut fields = vec!["  channel: Channel;".to_string()];
    fields.extend(config);
    fields.extend([
        format!("  resultHandler: (result: {result}) => void;"),
        "  errorHandler?: (error: any) => void;".to_string(),
        "  timeoutHandler?: () => void;".to_string(),
        "  timeout?: number;".to_string(),
    ]);
    let payload = payload(name, rpc, ctx, shape.has_fields, shape.has_metadata, true, true);
    format!(
        "{}\nexport async function {}{generics}(config: {{\n{}\n}}) {{\n  executeActionChannelPush<{result}>(\n    config.channel,\n    {payload},\n    config.timeout,\n    config\n  );\n}}\n",
        jsdoc(rpc.resource, rpc.action),
        to_camel_case(&format!("{name}_channel")),
        fields.join("\n"),
    )
}

fn validation_channel_function<D>(name: &str, pascal: &str, rpc: &RpcAction<D>, ctx: &Context, hooks: &Hooks) -> String {
    let mut fields = vec!["  channel: Channel;".to_string()];
    fields.extend(common_config(pascal, rpc, ctx, Request::ValidationChannel, hooks));
    fields.extend([
        "  resultHandler: (result: ValidationResult) => void;".to_string(),
        "  errorHandler?: (error: any) => void;".to_string(),
        "  timeoutHandler?: () => void;".to_string(),
        "  timeout?: number;".to_string(),
    ]);
    let payload = payload(name, rpc, ctx, false, false, false, false);
    format!(
        "{}\nexport async function {}(config: {{\n{}\n}}) {{\n  executeValidationChannelPush<ValidationResult>(\n    config.channel,\n    {payload},\n    config.timeout,\n    config\n  );\n}}\n",
        validation_jsdoc(rpc.resource, rpc.action),
        to_camel_case(&format!("validate_{name}_channel")),
        fields.join("\n"),
    )
}

fn rpc_function<D>(name: &str, rpc: &RpcAction<D>, config: &ClientConfig) -> String {
    let pascal = to_pascal_case(name);
    let ctx = context(rpc);
    let hooks = &config.hooks;
    let mut functions = vec![execution_function(name, &pascal, rpc, &ctx, hooks)];
    if config.validation_functions {
        functions.push(validation_function(name, &pascal, rpc, &ctx, hooks));
    }
    if config.validation_functions && config.channel_functions {
        functions.push(validation_channel_function(name, &pascal, rpc, &ctx, hooks));
    }
    if config.channel_functions {
        functions.push(channel_function(name, &pascal, rpc, &ctx, hooks));
    }
    let parts: Vec<String> = [input_type(&pascal, rpc), result_type(&pascal, rpc, &ctx), functions.join("\n\n")]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    parts.join("\n").trim_end_matches('\n').to_string() + "\n"
}

// ── Static helpers ──────────────────────────────────────────────────────────

fn hook_context_types(hooks: &Hooks) -> String {
    let mut parts = Vec::new();
    let mut add = |enabled: bool, ty: &Option<String>, label: &str, name: &str| {
        if let Some(ty) = ty.as_ref().filter(|ty| enabled && !ty.is_empty()) {
            parts.push(format!("// RPC {label} Hook Context Type\nexport type {name} = {ty};\n"));
        }
    };
    add(hooks.action(), &hooks.action_context_type, "Action", "ActionHookContext");
    add(hooks.validation(), &hooks.validation_context_type, "Validation", "ValidationHookContext");
    add(hooks.action_channel(), &hooks.action_channel_context_type, "Action Channel", "ActionChannelHookContext");
    add(hooks.validation_channel(), &hooks.validation_channel_context_type, "Validation Channel", "ValidationChannelHookContext");
    parts.join("\n").trim().to_string()
}

fn request_helper(function: &str, description: &str, config_type: &str, endpoint: &str, before: Option<&str>, after: Option<&str>) -> String {
    let before = match before {
        Some(hook) => format!(
            "    let processedConfig = config;\n    if ({hook}) {{\n      processedConfig = await {hook}(payload.action, config);\n    }}\n"
        ),
        None => "    const processedConfig = config;\n".to_string(),
    };
    let after = match after {
        Some(hook) => format!("    if ({hook}) {{\n      await {hook}(payload.action, response, result, processedConfig);\n    }}\n"),
        None => String::new(),
    };
    format!(
        r#"/**
 * Internal helper function for making {description}s
 * Handles hooks, request configuration, fetch execution, and error handling
 * @param config Configuration matching {config_type}
 */
export async function {function}<T>(
  payload: Record<string, any>,
  config: {config_type}
): Promise<T> {{
{before}
  const headers: Record<string, string> = {{
    "Content-Type": "application/json",
    ...processedConfig.headers,
    ...config.headers,
  }};

  const fetchFunction = config.customFetch || processedConfig.customFetch || fetch;
  const fetchOptions: RequestInit = {{
    ...processedConfig.fetchOptions,
    ...config.fetchOptions,
    method: "POST",
    headers,
    body: JSON.stringify(payload),
  }};

  const response = await fetchFunction({endpoint}, fetchOptions);
  const result = response.ok ? await response.json() : null;

{after}
  if (!response.ok) {{
    return {{
      success: false,
      errors: [
        {{
          type: "network_error",
          message: `Network request failed: ${{response.statusText}}`,
          shortMessage: "Network error",
          vars: {{ statusCode: response.status, statusText: response.statusText }},
          fields: [],
          path: [],
          details: {{ statusCode: response.status }}
        }}
      ],
    }} as T;
  }}

  return result as T;
}}
"#
    )
}

fn channel_helper(function: &str, description: &str, config_type: &str, event: &str, before: Option<&str>, after: Option<&str>) -> String {
    let before = match before {
        Some(hook) => format!(
            "    let processedConfig = config;\n    if ({hook}) {{\n      processedConfig = await {hook}(payload.action, config);\n    }}\n"
        ),
        None => "    const processedConfig = config;\n".to_string(),
    };
    let after = |status: &str, value: &str| match after {
        Some(hook) => format!("      if ({hook}) {{\n        await {hook}(payload.action, \"{status}\", {value}, processedConfig);\n      }}\n"),
        None => String::new(),
    };
    format!(
        r#"/**
 * Internal helper function for making {description} requests
 * Handles hooks and channel push with receive handlers
 * @param config Configuration matching {config_type}
 */
export async function {function}<T>(
  channel: any,
  payload: Record<string, any>,
  timeout: number | undefined,
  config: {config_type}
) {{
{before}
  const effectiveTimeout = timeout;

  channel
    .push("{event}", payload, effectiveTimeout)
    .receive("ok", async (result: T) => {{
{}
      config.resultHandler(result);
    }})
    .receive("error", async (error: any) => {{
{}
      (config.errorHandler
        ? config.errorHandler
        : (error: any) => {{
            console.error(
              `An error occurred while running action ${{payload.action}}:`,
              error
            );
          }})(error);
    }})
    .receive("timeout", async () => {{
{}
      (config.timeoutHandler
        ? config.timeoutHandler
        : () => {{
            console.error(`Timeout occurred while running action ${{payload.action}}`);
          }})();
    }});
}}
"#,
        after("ok", "result"),
        after("error", "error"),
        after("timeout", "undefined"),
    )
}

fn helper_functions(config: &ClientConfig) -> String {
    let hooks = &config.hooks;
    let context = |ty: &Option<String>| ty.clone().unwrap_or_else(|| "Record<string, any>".to_string());
    let validation_config = if config.validation_functions {
        format!(
            r#"
/**
 * Configuration options for validation RPC requests
 */
export interface ValidationConfig {{
  // Request data
  input?: Record<string, any> | undefined;

  // HTTP customization
  headers?: Record<string, string> | undefined;
  fetchOptions?: RequestInit | undefined;
  customFetch?: ((
    input: RequestInfo | URL,
    init?: RequestInit,
  ) => Promise<Response>) | undefined;

  // Hook context
  hookCtx?: {} | undefined;
}}
"#,
            context(&hooks.validation_context_type)
        )
    } else {
        String::new()
    };
    let channel_config = if config.channel_functions {
        format!(
            r#"
/**
 * Configuration options for action channel RPC requests
 */
export interface ActionChannelConfig {{
  // Request data
  input?: Record<string, any> | undefined;
  identity?: any;
  fields?: ReadonlyArray<string | Record<string, any>> | undefined;
  filter?: Record<string, any> | undefined;
  sort?: string | string[] | undefined;
  page?:
    | {{
        limit?: number;
        offset?: number;
        count?: boolean;
      }}
    | {{
        limit?: number;
        after?: string;
        before?: string;
      }}
    | undefined;

  // Metadata
  metadataFields?: ReadonlyArray<string> | undefined;

  // Channel-specific
  channel: any; // Phoenix Channel
  resultHandler: (result: any) => void;
  errorHandler?: ((error: any) => void) | undefined;
  timeoutHandler?: (() => void) | undefined;
  timeout?: number | undefined;

  // Multitenancy
  tenant?: string | undefined;

  // Hook context
  hookCtx?: {} | undefined;
}}
"#,
            context(&hooks.action_channel_context_type)
        )
    } else {
        String::new()
    };
    let validation_channel_config = if config.validation_functions && config.channel_functions {
        format!(
            r#"
/**
 * Configuration options for validation channel RPC requests
 */
export interface ValidationChannelConfig {{
  // Request data
  input?: Record<string, any> | undefined;
  identity?: any;

  // Channel-specific
  channel: any; // Phoenix Channel
  resultHandler: (result: any) => void;
  errorHandler?: ((error: any) => void) | undefined;
  timeoutHandler?: (() => void) | undefined;
  timeout?: number | undefined;

  // Multitenancy
  tenant?: string | undefined;

  // Hook context
  hookCtx?: {} | undefined;
}}
"#,
            context(&hooks.validation_channel_context_type)
        )
    } else {
        String::new()
    };
    let action_helper = request_helper(
        "executeActionRpcRequest",
        "action RPC request",
        "ActionConfig",
        &config.run_endpoint.ts(),
        hooks.action_before_request.as_deref(),
        hooks.action_after_request.as_deref(),
    );
    let validation_helper = if config.validation_functions {
        request_helper(
            "executeValidationRpcRequest",
            "validation RPC request",
            "ValidationConfig",
            &config.validate_endpoint.ts(),
            hooks.validation_before_request.as_deref(),
            hooks.validation_after_request.as_deref(),
        )
    } else {
        String::new()
    };
    let channel_helper_fn = if config.channel_functions {
        channel_helper(
            "executeActionChannelPush",
            "action channel push",
            "ActionChannelConfig",
            "run",
            hooks.action_before_channel_push.as_deref(),
            hooks.action_after_channel_response.as_deref(),
        )
    } else {
        String::new()
    };
    let validation_channel_helper = if config.validation_functions && config.channel_functions {
        channel_helper(
            "executeValidationChannelPush",
            "validation channel push",
            "ValidationChannelConfig",
            "validate",
            hooks.validation_before_channel_push.as_deref(),
            hooks.validation_after_channel_response.as_deref(),
        )
    } else {
        String::new()
    };
    format!(
        r#"// Helper Functions

/**
 * Configuration options for action RPC requests
 */
export interface ActionConfig {{
  // Request data
  input?: Record<string, any> | undefined;
  identity?: any;
  fields?: Array<string | Record<string, any>> | undefined; // Field selection
  filter?: Record<string, any> | undefined; // Filter options (for reads)
  sort?: string | string[] | undefined; // Sort options
  page?:
    | {{
        // Offset-based pagination
        limit?: number;
        offset?: number;
        count?: boolean;
      }}
    | {{
        // Keyset pagination
        limit?: number;
        after?: string;
        before?: string;
      }}
    | undefined;

  // Metadata
  metadataFields?: ReadonlyArray<string> | undefined;

  // HTTP customization
  headers?: Record<string, string> | undefined; // Custom headers
  fetchOptions?: RequestInit | undefined; // Fetch options (signal, cache, etc.)
  customFetch?: ((
    input: RequestInfo | URL,
    init?: RequestInit,
  ) => Promise<Response>) | undefined;

  // Multitenancy
  tenant?: string | undefined; // Tenant parameter

  // Hook context
  hookCtx?: {} | undefined;
}}
{validation_config}
{channel_config}
{validation_channel_config}

/**
 * Gets the CSRF token from the page's meta tag
 * Returns null if no CSRF token is found
 */
export function getPhoenixCSRFToken(): string | null {{
  return document
    ?.querySelector("meta[name='csrf-token']")
    ?.getAttribute("content") || null;
}}

/**
 * Builds headers object with CSRF token for Phoenix applications
 * Returns headers object with X-CSRF-Token (if available)
 */
export function buildCSRFHeaders(headers: Record<string, string> = {{}}): Record<string, string> {{
  const csrfToken = getPhoenixCSRFToken();
  if (csrfToken) {{
    headers["X-CSRF-Token"] = csrfToken;
  }}

  return headers;
}}

{action_helper}

{validation_helper}

{channel_helper_fn}

{validation_channel_helper}
"#,
        context(&hooks.action_context_type)
    )
}

// ── Typed queries ──────────────────────────────────────────────────────────

fn typed_query_fields(resource: &ResourceDef, fields: &serde_json::Value) -> String {
    match fields {
        serde_json::Value::Array(items) => format!("[{}]", items.iter().map(|item| typed_query_field(resource, item)).collect::<Vec<_>>().join(", ")),
        other => typed_query_field(resource, other),
    }
}

fn typed_query_field(resource: &ResourceDef, field: &serde_json::Value) -> String {
    match field {
        serde_json::Value::String(name) => format!("\"{}\"", to_camel_case(name)),
        serde_json::Value::Object(map) => {
            let mut pairs: Vec<(&String, &serde_json::Value)> = map.iter().collect();
            pairs.sort_by_key(|(key, _)| key.as_str());
            let pairs: Vec<String> = pairs
                .into_iter()
                .map(|(key, value)| {
                    let nested = resource
                        .relationship(key)
                        .map_or(resource, |rel| (rel.destination)());
                    format!("{}: {}", to_camel_case(key), typed_query_fields(nested, value))
                })
                .collect();
            format!("{{ {} }}", pairs.join(", "))
        }
        other => other.to_string(),
    }
}

fn typed_queries_section<D>(queries: &[TypedQuery], actions: &[(&str, &RpcAction<D>)]) -> String {
    if queries.is_empty() {
        return String::new();
    }
    let mut by_resource: Vec<(&str, Vec<&TypedQuery>)> = Vec::new();
    for query in queries {
        match by_resource.iter_mut().find(|(name, _)| *name == query.resource.name) {
            Some((_, list)) => list.push(query),
            None => by_resource.push((query.resource.name, vec![query])),
        }
    }
    by_resource.sort_by_key(|(name, _)| *name);
    let sections: Vec<String> = by_resource
        .iter()
        .map(|(name, queries)| {
            let rendered: Vec<String> = queries
                .iter()
                .map(|query| {
                    let fields = typed_query_fields(query.resource, &query.fields);
                    let type_name = query.result_type_name.clone().unwrap_or_else(|| format!("{}Result", to_pascal_case(&query.name)));
                    let const_name = query.fields_const_name.clone().unwrap_or_else(|| to_camel_case(&query.name));
                    let schema = format!("{}ResourceSchema", query.resource.name);
                    let result = if query.action.kind == ActionKind::Read {
                        format!("Array<InferResult<{schema}, {fields}>>")
                    } else {
                        format!("InferResult<{schema}, {fields}>")
                    };
                    // The fields type of an RPC action over the same read: a list's first.
                    let mut matching: Vec<&(&str, &RpcAction<D>)> = actions
                        .iter()
                        .filter(|(_, rpc)| rpc.resource.name == query.resource.name && rpc.action.name == query.action.name)
                        .collect();
                    matching.sort_by_key(|(name, rpc)| (rpc.options.get || !rpc.options.get_by.is_empty(), name.to_string()));
                    let satisfies = matching
                        .first()
                        .map(|(name, _)| format!(" satisfies {}Fields", to_pascal_case(name)))
                        .unwrap_or_default();
                    let description = query
                        .description
                        .clone()
                        .filter(|description| !description.is_empty())
                        .unwrap_or_else(|| format!("Typed query for {name}"));
                    let jsdoc = format!("/**\n * {description}\n *\n * @typedQuery true\n */");
                    format!("{jsdoc}\nexport type {type_name} = {result};\n\n{jsdoc}\nexport const {const_name} = {fields}{satisfies};\n")
                })
                .collect();
            format!("// {name} Typed Queries\n{}\n", rendered.join("\n\n"))
        })
        .collect();
    format!(
        "// ============================\n// Typed Queries\n// ============================\n// Use these types and field constants for server-side rendering and data fetching.\n// The field constants can be used with the corresponding RPC actions for client-side refetching.\n\n{}\n",
        sections.join("\n\n")
    )
}

/// The names `body` uses of those `types` exports.
fn used_type_names(types: &str, body: &str) -> Vec<String> {
    let mut exported: Vec<String> = types
        .lines()
        .filter_map(|line| line.strip_prefix("export type ").or_else(|| line.strip_prefix("export interface ")))
        .filter_map(|rest| rest.split(|c: char| !(c.is_alphanumeric() || c == '_')).next())
        .map(str::to_string)
        .collect();
    exported.sort();
    exported.dedup();
    let words: std::collections::HashSet<&str> = body.split(|c: char| !(c.is_alphanumeric() || c == '_')).collect();
    exported.into_iter().filter(|name| words.contains(name.as_str())).collect()
}

fn rpc_file<D>(actions: &[(&str, &RpcAction<D>)], queries: &[TypedQuery], types: &str, config: &ClientConfig) -> String {
    let functions: Vec<String> = actions.iter().map(|(name, rpc)| rpc_function(name, rpc, config)).collect();
    let body: Vec<String> = [hook_context_types(&config.hooks), helper_functions(config), typed_queries_section(queries, actions), functions.join("\n\n")]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect();
    let body = body.join("\n\n");
    let used = used_type_names(types, &body);
    let import = config.types_import.as_str();
    let imports = if config.channel_functions { format!("import {{ Channel }} from \"{}\";\n", config.phoenix_import) } else { String::new() };
    format!(
        "// Generated by AshTypescript - RPC Actions\n// Do not edit this file manually\n\n{imports}\nimport type {{ {} }} from \"{import}\";\nexport type * from \"{import}\";\n\n{body}\n",
        used.join(", ")
    )
}
