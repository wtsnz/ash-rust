//! Errors as AshTypescript reports them: `type`, `message` (a template, its `%{var}`s
//! left for the client to fill), `shortMessage`, `vars`, `fields`, `path`, and for a
//! request it couldn't make sense of, `details` with a suggestion.

use ash_core::{Error, ResourceDef};
use serde_json::{Map, Value as Json, json};

use crate::types::to_camel_case;

/// What AshTypescript adds to each error a request's shape causes.
const STALE_CLIENT_HINT: &str = "This error is most likely happening because the generated typescript file used is not up to date with the running backend. Check that you are using the latest generated file, and/or that a new file has been generated after the last backend changes.";

/// An error, as AshTypescript reports one.
#[derive(Debug, Clone)]
pub struct Failure(Box<Shape>);

/// What an error reports.
#[derive(Debug, Clone)]
pub struct Shape {
    pub kind: &'static str,
    pub message: String,
    pub short: &'static str,
    pub vars: Map<String, Json>,
    pub fields: Vec<String>,
    /// Where it is: field names, or a record's index in a bulk action.
    pub path: Vec<Json>,
    pub details: Option<Map<String, Json>>,
    pub error_id: Option<String>,
    /// The errors reported with it, as Ash reports every validation a changeset fails.
    pub rest: Vec<Failure>,
}

/// A field as the client names it: `comments.authorId`, its parents' path first.
pub(crate) fn field_path(path: &[String], field: &str) -> String {
    let field = to_camel_case(field);
    if path.is_empty() { field } else { format!("{}.{field}", path.join(".")) }
}

fn vars(pairs: &[(&str, Json)]) -> Map<String, Json> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

impl std::ops::Deref for Failure {
    type Target = Shape;

    fn deref(&self) -> &Shape {
        &self.0
    }
}

impl std::ops::DerefMut for Failure {
    fn deref_mut(&mut self) -> &mut Shape {
        &mut self.0
    }
}

impl Failure {
    pub(crate) fn new(kind: &'static str, short: &'static str, message: impl Into<String>) -> Self {
        Self(Box::new(Shape {
            kind,
            message: message.into(),
            short,
            vars: Map::new(),
            fields: Vec::new(),
            path: Vec::new(),
            details: None,
            error_id: None,
            rest: Vec::new(),
        }))
    }

    fn vars(mut self, pairs: &[(&str, Json)]) -> Self {
        self.vars = vars(pairs);
        self
    }

    fn fields(mut self, fields: Vec<String>) -> Self {
        self.fields = fields;
        self
    }

    fn at(mut self, path: &[String]) -> Self {
        self.path = path.iter().map(|part| json!(part)).collect();
        self
    }

    /// Details for a request AshTypescript rejects before running it, with its hint
    /// that the client may be out of date.
    fn details(mut self, pairs: &[(&str, Json)]) -> Self {
        let mut details = vars(pairs);
        details.insert("hint".into(), json!(STALE_CLIENT_HINT));
        self.details = Some(details);
        self
    }

    /// A field-level error at `path`: the field named as the client names it, its path
    /// as the client's.
    fn on_field(self, path: &[String], field: &str) -> Self {
        let full = field_path(path, field);
        self.fields(vec![full]).at(&path.iter().map(|p| to_camel_case(p)).collect::<Vec<_>>())
    }

    pub(crate) fn action_not_found(name: &str) -> Self {
        Self::new("action_not_found", "Action not found", "RPC action %{actionName} not found")
            .vars(&[("actionName", json!(name))])
            .details(&[("suggestion", json!("Check that the action is properly configured in your domain's rpc block"))])
    }

    pub(crate) fn missing_required_parameter(parameter: &str) -> Self {
        Self::new("missing_required_parameter", "Missing required parameter", "Required parameter %{parameter} is missing or empty")
            .vars(&[("parameter", json!(parameter))])
            .details(&[("suggestion", json!("Ensure %{parameter} parameter is provided and not empty"))])
    }

    pub(crate) fn invalid_fields_type(received: &Json) -> Self {
        Self::new("invalid_fields_type", "Invalid fields type", "Fields parameter must be an array")
            .vars(&[("received", json!(received.to_string()))])
            .details(&[
                ("expectedCode", json!("array")),
                ("suggestion", json!("Wrap field names in an array, e.g., [\"field1\", \"field2\"]")),
            ])
    }

    pub(crate) fn empty_fields_array() -> Self {
        Self::new("empty_fields_array", "Empty fields array", "Fields array cannot be empty")
            .details(&[("suggestion", json!("Provide at least one field name in the fields array"))])
    }

    pub(crate) fn unknown_field(path: &[String], field: &str, resource: &ResourceDef) -> Self {
        let full = field_path(path, field);
        Self::new("unknown_field", "Unknown field", "Unknown field %{field} for resource %{resource}")
            .vars(&[("field", json!(full)), ("resource", json!(resource.name))])
            .on_field(path, field)
            .details(&[(
                "suggestion",
                json!("Check the field name spelling and ensure it's a public attribute, calculation, or relationship"),
            )])
    }

    pub(crate) fn duplicate_field(path: &[String], field: &str) -> Self {
        Self::new("duplicate_field", "Duplicate field", "Field %{field} was requested multiple times")
            .vars(&[("field", json!(field_path(path, field)))])
            .on_field(path, field)
            .details(&[("suggestion", json!("Remove duplicate field specifications"))])
    }

    /// A field that needs a selection got none: a relationship, or a calculation of a
    /// type that has fields.
    pub(crate) fn requires_field_selection(path: &[String], field: &str, field_type: &str) -> Self {
        Self::new("requires_field_selection", "Field selection required", "%{fieldType} %{field} requires field selection")
            .vars(&[("fieldType", json!(capitalize(field_type))), ("field", json!(field_path(path, field)))])
            .on_field(path, field)
            .details(&[("suggestion", json!(format!("Specify which fields to select from this {field_type}")))])
    }

    pub(crate) fn field_does_not_support_nesting(path: &[String], field: &str) -> Self {
        Self::new("field_does_not_support_nesting", "Field does not support nesting", "Field %{field} does not support nested field selection")
            .vars(&[("field", json!(field_path(path, field)))])
            .on_field(path, field)
            .details(&[("suggestion", json!("Remove the nested specification for this field"))])
    }

    pub(crate) fn invalid_field_selection(path: &[String], field: &str, field_type: &str) -> Self {
        Self::new("invalid_field_selection", "Invalid field selection", "Cannot select fields from %{fieldType} %{field}")
            .vars(&[("fieldType", json!(field_type)), ("field", json!(field_path(path, field)))])
            .on_field(path, field)
            .details(&[("suggestion", json!(format!("Remove the field selection for this {field_type} field")))])
    }

    pub(crate) fn calculation_requires_args(path: &[String], field: &str) -> Self {
        Self::new("invalid_field_format", "Calculation requires arguments", "Calculation %{field} requires arguments")
            .vars(&[("field", json!(field_path(path, field)))])
            .on_field(path, field)
            .details(&[("suggestion", json!(format!("Provide arguments in the format: {{\"{field}\": {{\"args\": {{...}}}}}}")))])
    }

    pub(crate) fn invalid_calculation_args(path: &[String], field: &str) -> Self {
        Self::new("invalid_calculation_args", "Invalid calculation arguments", "Invalid arguments for calculation %{field}")
            .vars(&[("field", json!(field_path(path, field)))])
            .on_field(path, field)
            .details(&[("expected", json!("Map containing argument values or valid field selection format"))])
    }

    pub(crate) fn unsupported_field_combination(path: &[String], field: &str, field_type: &str, spec: &Json) -> Self {
        Self::new(
            "unsupported_field_combination",
            "Unsupported field combination",
            "Unsupported combination of field type and specification for %{field}",
        )
        .vars(&[("field", json!(field_path(path, field))), ("fieldType", json!(field_type))])
        .on_field(path, field)
        .details(&[
            ("fieldSpec", json!(spec.to_string())),
            ("suggestion", json!("Check the documentation for valid field specification formats")),
        ])
    }

    /// An item of a selection that's neither a name nor an object, as AshTypescript reports
    /// it.
    pub(crate) fn invalid_field_item(path: &[String], item: &Json) -> Self {
        let name = match item {
            Json::String(text) => text.clone(),
            other => other.to_string(),
        };
        let mut parts: Vec<String> = path.iter().map(|p| to_camel_case(p)).collect();
        parts.push(name);
        let full = parts.join(".");
        Self::new("unknown_field", "Unknown field", "Unknown field %{field}")
            .vars(&[("field", json!(full))])
            .fields(vec![full])
            .at(&path.iter().map(|p| to_camel_case(p)).collect::<Vec<_>>())
            .details(&[("suggestion", json!("Check that the field exists and is accessible"))])
    }

    pub(crate) fn invalid_field_format(path: &[String], spec: &Json) -> Self {
        Self::new("invalid_field_format", "Invalid field format", "Invalid field specification format")
            .at(&path.iter().map(|p| to_camel_case(p)).collect::<Vec<_>>())
            .details(&[
                ("fieldSpec", json!(spec.to_string())),
                ("suggestion", json!("Check the documentation for valid field specification formats")),
            ])
    }

    pub(crate) fn query_opts_on_non_relationship(path: &[String], field: &str, kind: &str) -> Self {
        Self::new(
            "invalid_query_opts",
            "Invalid query options",
            "Field %{field} is a %{kind} and does not accept query options (page/filter/sort/limit/offset)",
        )
        .vars(&[("field", json!(field_path(path, field))), ("kind", json!(kind))])
        .on_field(path, field)
        .details(&[("suggestion", json!("Query options are only supported on has_many and many_to_many relationships"))])
    }

    pub(crate) fn query_opts_on_to_one(path: &[String], field: &str) -> Self {
        Self::new("invalid_query_opts", "Invalid query options", "Relationship %{field} is to-one and does not accept query options")
            .vars(&[("field", json!(field_path(path, field)))])
            .on_field(path, field)
            .details(&[("suggestion", json!("Use a plain nested field list for to-one relationships"))])
    }

    pub(crate) fn args_and_query_opts_combined(path: &[String], field: &str) -> Self {
        Self::new(
            "invalid_query_opts",
            "Invalid query options",
            "Field %{field} combines args with query options; they are mutually exclusive",
        )
        .vars(&[("field", json!(field_path(path, field)))])
        .on_field(path, field)
        .details(&[("suggestion", json!("Remove the args key — relationships do not take arguments"))])
    }

    pub(crate) fn page_and_limit_offset_combined(path: &[String], field: &str) -> Self {
        Self::new(
            "invalid_query_opts",
            "Invalid query options",
            "Field %{field} combines page with bare limit/offset; use one or the other",
        )
        .vars(&[("field", json!(field_path(path, field)))])
        .on_field(path, field)
        .details(&[("suggestion", json!("Use page for paginated results or bare limit/offset for a plain slice"))])
    }

    pub(crate) fn nested_pagination_not_supported(path: &[String], field: &str) -> Self {
        Self::new("pagination_not_supported", "Pagination not supported", "Relationship %{field} does not support pagination")
            .vars(&[("field", json!(field_path(path, field)))])
            .on_field(path, field)
            .details(&[
                ("reason", json!("unsupported")),
                (
                    "suggestion",
                    json!("The relationship's read action has no pagination configured. Use bare limit/offset, or add pagination to the destination read action"),
                ),
            ])
    }

    pub(crate) fn invalid_nested_page(path: &[String], field: &str, reason: &str) -> Self {
        Self::new("invalid_pagination", "Invalid pagination", "Invalid page configuration for relationship %{field}")
            .vars(&[("field", json!(field_path(path, field)))])
            .on_field(path, field)
            .details(&[
                ("reason", json!(reason)),
                (
                    "suggestion",
                    json!("Provide page keys valid for the relationship's pagination type (offset: limit/offset/count, keyset: limit/after/before/count)"),
                ),
            ])
    }

    /// A filter, sort or page the action can't take: `unsupported` by its kind, or
    /// `disabled` by the RPC action's options. `field` names a relationship's.
    pub(crate) fn option_not_supported(option: &str, reason: &str, at: Option<(&[String], &str)>) -> Self {
        let (kind, short) = match option {
            "filter" => ("filter_not_supported", "Filter not supported"),
            "sort" => ("sort_not_supported", "Sort not supported"),
            _ => ("pagination_not_supported", "Pagination not supported"),
        };
        match at {
            None => {
                let (message, suggestion) = match option {
                    "filter" => (
                        "This action does not support the filter parameter",
                        "Remove the filter parameter. It is unavailable because the action is not a list read (get?/non-read) or filtering is disabled via enable_filter?: false",
                    ),
                    "sort" => (
                        "This action does not support the sort parameter",
                        "Remove the sort parameter. It is unavailable because the action is not a list read (get?/non-read) or sorting is disabled via enable_sort?: false",
                    ),
                    _ => (
                        "This action does not support the page parameter",
                        "Remove the page parameter. It is unavailable because the action is not a list read (get?/non-read) or has no pagination configured",
                    ),
                };
                Self::new(kind, short, message).details(&[("reason", json!(reason)), ("suggestion", json!(suggestion))])
            }
            Some((path, field)) => {
                let (message, suggestion) = match option {
                    "filter" => (
                        "Relationship %{field} does not support filtering",
                        "Remove the filter key. Filtering is unavailable because the RPC action disables it (enable_filter?: false) or the relationship is not filterable?",
                    ),
                    _ => (
                        "Relationship %{field} does not support sorting",
                        "Remove the sort key. Sorting is unavailable because the RPC action disables it (enable_sort?: false) or the relationship is not sortable?",
                    ),
                };
                Self::new(kind, short, message)
                    .vars(&[("field", json!(field_path(path, field)))])
                    .on_field(path, field)
                    .details(&[("reason", json!(reason)), ("suggestion", json!(suggestion))])
            }
        }
    }

    pub(crate) fn load_restricted(denied: bool, paths: Vec<String>) -> Self {
        let joined = json!(paths.join(", "));
        let (kind, short, message, key, suggestion) = if denied {
            ("load_denied", "Load denied", "Loading the following fields is denied: %{fields}", "deniedPaths", "Remove these fields from your request")
        } else {
            (
                "load_not_allowed",
                "Load not allowed",
                "Loading the following fields is not allowed: %{fields}",
                "disallowedPaths",
                "Remove these fields from your request or check allowed_loads configuration",
            )
        };
        Self::new(kind, short, message)
            .vars(&[("fields", joined)])
            .fields(paths.clone())
            .details(&[(key, json!(paths)), ("suggestion", json!(suggestion))])
    }

    pub(crate) fn invalid_input_format(received: &Json) -> Self {
        Self::new("invalid_input_format", "Invalid input format", "Input parameter must be a map")
            .vars(&[("received", json!(inspect(received)))])
            .details(&[("expected", json!("Map containing input parameters"))])
    }

    pub(crate) fn missing_get_by_fields(missing: Vec<String>) -> Self {
        Self::new("missing_required_input", "Missing required getBy fields", "Required getBy fields are missing: %{fields}")
            .vars(&[("fields", json!(missing.join(", ")))])
            .fields(missing)
            .at(&["get_by".to_string()])
            .details(&[("suggestion", json!("Provide values for all required getBy fields"))])
    }

    pub(crate) fn unexpected_get_by_fields(extra: Vec<String>, allowed: Vec<String>) -> Self {
        Self::new(
            "unexpected_get_by_fields",
            "Unexpected getBy fields",
            "Unexpected getBy fields: %{extraFields}. Allowed fields: %{allowedFields}",
        )
        .vars(&[("extraFields", json!(extra.join(", "))), ("allowedFields", json!(allowed.join(", ")))])
        .fields(extra)
        .at(&["get_by".to_string()])
        .details(&[("allowedFields", json!(allowed)), ("suggestion", json!("Only provide the allowed getBy fields: %{allowedFields}"))])
    }

    pub(crate) fn invalid_get_by(fields: &[String]) -> Self {
        Self::new(
            "invalid_get_by",
            "Invalid getBy value",
            format!("getBy values must be scalar equality operands. Non-scalar value provided for: {}", fields.join(", ")),
        )
        .at(&["get_by".to_string()])
        .details(&[("suggestion", json!("Provide a scalar value for each getBy field"))])
    }

    pub(crate) fn unknown_page_keys(keys: Vec<String>) -> Self {
        Self::new("invalid_pagination", "Invalid pagination", "Unknown pagination keys: %{keys}")
            .vars(&[("keys", json!(keys.join(", ")))])
            .fields(keys)
            .at(&["page".to_string()])
            .details(&[("expected", json!("Offset pagination (limit, offset, count) or keyset pagination (limit, after, before)"))])
    }

    pub(crate) fn invalid_pagination(received: &Json) -> Self {
        Self::new("invalid_pagination", "Invalid pagination", "Invalid pagination parameter format")
            .vars(&[("received", json!(inspect(received)))])
            .details(&[("expected", json!("Map with pagination parameters (limit, offset, before, after, etc.)"))])
    }

    pub(crate) fn missing_identity(expected: Vec<String>, primary_key_only: bool) -> Self {
        let joined = expected.join(", ");
        let (message, suggestion) = match (primary_key_only, expected.as_slice()) {
            (true, [key]) => (
                format!("Identity is required. Provide the {key} value directly."),
                format!("Pass the {key} value directly as the identity field (e.g., identity: \"your-{key}-here\")"),
            ),
            _ => (
                "Identity is required but not provided. Expected one of: [%{expectedKeys}]".to_string(),
                format!("Provide identity fields for one of the configured identities: {joined}"),
            ),
        };
        Self::new("missing_identity", "Missing identity", message)
            .vars(&[("expectedKeys", json!(joined))])
            .at(&["identity".to_string()])
            .details(&[("expectedKeys", json!(expected)), ("suggestion", json!(suggestion))])
    }

    pub(crate) fn invalid_identity(provided: Vec<String>, expected: Vec<String>) -> Self {
        let (provided_joined, expected_joined) = (provided.join(", "), expected.join(", "));
        Self::new(
            "invalid_identity",
            "Invalid identity",
            "Identity fields do not match any configured identity. Provided: [%{providedKeys}], expected: [%{expectedKeys}]",
        )
        .vars(&[("providedKeys", json!(provided_joined)), ("expectedKeys", json!(expected_joined))])
        .at(&["identity".to_string()])
        .details(&[
            ("providedKeys", json!(provided)),
            ("expectedKeys", json!(expected)),
            ("suggestion", json!(format!("Provide all required fields for one of the configured identities: {expected_joined}"))),
        ])
    }

    pub(crate) fn invalid_identity_value(message: impl Into<String>) -> Self {
        Self::new("invalid_identity", "Invalid identity", message)
            .at(&["identity".to_string()])
            .details(&[("suggestion", json!("Check the configured identities for this action"))])
    }

    /// A problem a form validating input finds, as AshTypescript reports it: an invalid
    /// attribute at its field, its message filled in.
    pub(crate) fn form_error(field: &str, message: String) -> Self {
        Self::new("invalid_attribute", "Invalid attribute", message).vars(&[("field", json!(field))]).fields(vec![field.to_string()]).at(&[field.to_string()])
    }

    /// An error AshTypescript has no shape for: reported by its id alone, the error itself
    /// left to the server's [`Rpc::on_error`](super::Rpc::on_error).
    pub(crate) fn internal(error_id: String) -> Self {
        let mut failure = Self::new("internal_error", "Internal error", format!("Something went wrong. Unique error id: {error_id}"));
        failure.error_id = Some(error_id);
        failure
    }

    pub(crate) fn invalid_keyset(value: &str, direction: &str) -> Self {
        Self::new("invalid_keyset", "Invalid keyset", format!("Invalid value provided as a keyset for {direction}: {value:?}"))
    }

    /// This error found under `path`, where a filter or sort put it.
    pub(crate) fn under(mut self, path: &str) -> Self {
        if self.path.is_empty() {
            self.path = vec![json!(path)];
        }
        self
    }

    /// This error raised for the first record of a bulk action, as AshTypescript's
    /// updates (run as `Ash.bulk_update`) report theirs.
    pub(crate) fn in_bulk(mut self) -> Self {
        if self.path.is_empty() {
            self.path = vec![json!(0)];
        }
        self.rest = std::mem::take(&mut self.rest).into_iter().map(Failure::in_bulk).collect();
        self
    }

    /// A required field's error naming what it is, an attribute or an argument, as Ash's
    /// does.
    pub(crate) fn required_as(mut self, argument: bool) -> Self {
        if self.kind == "required"
            && let Some(field) = self.message.strip_prefix("argument ").and_then(|m| m.strip_suffix(" is required"))
        {
            let what = if argument { "argument" } else { "attribute" };
            self.message = format!("{what} {field} is required");
        }
        self.rest = std::mem::take(&mut self.rest).into_iter().map(|failure| failure.required_as(argument)).collect();
        self
    }

    /// This error and those reported with it, each on its own.
    pub fn flatten(mut self) -> Vec<Failure> {
        let rest = std::mem::take(&mut self.rest);
        std::iter::once(self).chain(rest.into_iter().flat_map(Failure::flatten)).collect()
    }

    pub fn to_json(&self) -> Json {
        let mut out = Map::new();
        out.insert("type".into(), json!(self.kind));
        out.insert("message".into(), json!(self.message));
        out.insert("shortMessage".into(), json!(self.short));
        out.insert("vars".into(), Json::Object(self.vars.clone()));
        out.insert("fields".into(), json!(self.fields));
        out.insert("path".into(), json!(self.path));
        if let Some(details) = &self.details {
            out.insert("details".into(), Json::Object(details.clone()));
        }
        if let Some(id) = &self.error_id {
            out.insert("errorId".into(), json!(id));
        }
        Json::Object(out)
    }

    /// An ash-core error as AshTypescript reports Ash's equivalent, through its error
    /// protocol. One it has no shape for is `None`: an internal error.
    pub(crate) fn from_error(err: &Error) -> Option<Self> {
        Some(match err {
            Error::NotFound => Self::new("not_found", "Not found", "record not found"),
            Error::Forbidden => Self::new("forbidden", "Forbidden", "forbidden"),
            Error::TenantRequired { resource } => Self::new("tenant_required", "Tenant required", err.to_string())
                .vars(&[("resource", json!(resource))]),
            // A validation's message a template, its vars beside it, as Ash's.
            Error::Validation { field, message, vars } => {
                let field = to_camel_case(field);
                // Its vars named as the client names fields, in the template too.
                let message = vars.iter().fold(message.clone(), |message, (name, _)| {
                    message.replace(&format!("%{{{name}}}"), &format!("%{{{}}}", to_camel_case(name)))
                });
                let mut failure = Self::new("invalid_attribute", "Invalid attribute", message).fields(vec![field.clone()]);
                failure.vars = vars.iter().map(|(name, value)| (to_camel_case(name), value.to_plain_json())).collect();
                failure.vars.insert("field".into(), json!(field));
                failure
            }
            Error::Constraint { field, message } => {
                let field = to_camel_case(field);
                Self::new("invalid_attribute", "Invalid attribute", message.clone())
                    .vars(&[("field", json!(field))])
                    .fields(vec![field])
            }
            // A value of the wrong type is invalid, as Ash finds it casting the input.
            Error::TypeMismatch { field, .. } => {
                let field = to_camel_case(field);
                Self::new("invalid_attribute", "Invalid attribute", "is invalid")
                    .vars(&[("field", json!(field))])
                    .fields(vec![field])
            }
            // An identity taken already, on its first field, as Ash reports it.
            Error::IdentityConflict { fields, message, .. } => {
                let field = to_camel_case(fields.first().map_or("", String::as_str));
                Self::new("invalid_attribute", "Invalid attribute", message.clone())
                    .vars(&[("field", json!(field))])
                    .fields(vec![field])
            }
            Error::Multi { source, .. } => return Self::from_error(source),
            Error::Missing { field } => {
                let camel = to_camel_case(field);
                Self::new("required", "Required field", format!("argument {field} is required"))
                    .vars(&[("field", json!(camel))])
                    .fields(vec![camel])
            }
            // AshTypescript has no shape for Ash's `NoSuchInput`, nor for a malformed
            // filter, which it reports as internal errors; ash-rust says what's wrong.
            Error::NotAccepted { field, action } => {
                let camel = to_camel_case(field);
                Self::new("invalid_argument", "Invalid argument", format!("No such input `{field}` for action {action}"))
                    .vars(&[("field", json!(camel))])
                    .fields(vec![camel])
            }
            Error::Invalid(message) => Self::new("invalid", "Invalid", message.clone()),
            _ => return None,
        })
    }
}

/// `"requesterEmail"` → `"Requester email"`-style: the first letter upper-cased, as
/// Elixir's `String.capitalize/1` writes a field type.
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars.flat_map(char::to_lowercase)).collect(),
        None => String::new(),
    }
}

/// A JSON value as Elixir's `inspect/1` writes the term it decodes to.
fn inspect(value: &Json) -> String {
    match value {
        Json::Null => "nil".into(),
        Json::String(s) => format!("{s:?}"),
        other => other.to_string(),
    }
}
