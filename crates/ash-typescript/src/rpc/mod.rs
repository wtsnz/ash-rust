//! AshTypescript's RPC: actions run by name over one endpoint, wire-compatible with
//! `AshTypescript.Rpc.run_action`, so a client AshTypescript generates works against an
//! ash-rust server as against an Elixir one.
//!
//! A request is JSON: `action` (the RPC action's name), and as the action needs them
//! `fields`, `input`, `identity`, `getBy`, `filter`, `sort`, `page` and `tenant`. Field
//! names are camelCase on the wire, snake_case in the resource. The answer is always
//! `{"success": true, "data": …}` or `{"success": false, "errors": [{"type", "message",
//! "shortMessage", "vars", "fields", "path", "details"?}]}`.
//!
//! - `fields` selects what comes back (see [`fields`]): attributes, aggregates and
//!   calculations, relationships with fields of their own, or their own read
//!   (`{"comments": {"fields": ["body"], "filter": …, "sort": "-insertedAt", "page": …}}`),
//!   and calculations with arguments (`{"label": {"args": {…}}}`). A read must select
//!   something; a write needn't.
//! - A read answers a list, or a page as its action's `pagination` decides: an offset
//!   page (`limit`, `offset`, `count`) or a keyset page (`after`, `before`, `nextPage`,
//!   `previousPage`). A get (`get_by`) answers one record, or `not_found`.
//! - A create, update or destroy answers the selected fields of the record (`{}` with no
//!   `fields`). An update or destroy finds it by `identity` (its primary key, or one of
//!   its identities' fields), through the read action as the actor: one it can't read
//!   is `not_found`, but a destroy that finds nothing answers `{}`, as AshTypescript's
//!   bulk destroy does.
//! - A generic action runs as Ash runs one, authorized by its policies, and answers what
//!   it returns.
//!
//! A request AshTypescript would refuse before running (a read without fields, a filter
//! on a get, an unknown page key, a field selected twice) is refused here the same way,
//! in the same order, with the same error.
//!
//! Reads run through ash-core as every other read does: the action's preparations, the
//! actor's policies and the tenant, relationship loads with their own policies, a
//! client's filters and sorts guarded by field policies, and fields the actor may not
//! read redacted.

mod error;
pub mod fields;
mod codegen;
mod names;

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ash_core::input::{action_input, field_type, filter_input, sort_input, value_input};
use ash_core::{
    ActionDef, ActionKind, AttrType, CompiledQuery, Context, Countable, DataLayer, Error, FieldMap, Filter,
    KeysetCursor, MetadataDef, PreparationDef, RelKind, RelatedQuery, Resource, ResourceDef, Sort, TransactionSupport, Value,
};
use serde_json::{Map, Value as Json, json};

pub use codegen::{Client, ClientConfig, Endpoint, Hooks};
pub use error::Failure;
pub use fields::LoadRestrictions;
use fields::{NestedPage, Rules, Selection, ValueSelection, sort_text};
use names::{Names, snake};

use crate::types::to_camel_case;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a generic action does, supplied with its RPC action: the context it runs in, and
/// its input, cast to its arguments' types.
pub type GenericRunner<D> = Arc<dyn Fn(Context<D>, FieldMap) -> BoxFuture<'static, ash_core::Result<Value>> + Send + Sync>;

/// Told of each error AshTypescript has no shape for, with the id its client was given.
pub type ErrorReporter = Arc<dyn Fn(&str, &Error) + Send + Sync>;

/// Rewrites an error before the client sees it, or drops it (`None`), as AshTypescript's
/// `error_handler` does, given the RPC action it came from.
pub type ErrorHandler = Arc<dyn Fn(Failure, &ErrorSource) -> Option<Failure> + Send + Sync>;

/// Where an error came from: the RPC action's resource and action, when the request
/// named one.
#[derive(Clone, Debug, Default)]
pub struct ErrorSource {
    pub rpc_action: Option<String>,
    pub resource: Option<&'static str>,
    pub action: Option<&'static str>,
}

/// How an update or destroy names its record: by primary key, or by one of the
/// resource's identities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Identity {
    PrimaryKey,
    Named(&'static str),
}

/// An RPC action's options, as `rpc_action` takes them.
#[derive(Clone, Debug)]
pub struct ActionOptions {
    get: bool,
    get_by: Vec<&'static str>,
    not_found_error: Option<bool>,
    read_action: Option<&'static str>,
    identities: Vec<Identity>,
    enable_filter: bool,
    enable_sort: bool,
    loads: LoadRestrictions,
    /// The action metadata a client may ask for: `None` for all of it.
    show_metadata: Option<Vec<&'static str>>,
    /// The names clients know metadata fields by, where not their camelCase.
    metadata_field_names: Vec<(&'static str, &'static str)>,
}

impl Default for ActionOptions {
    fn default() -> Self {
        Self {
            get: false,
            get_by: Vec::new(),
            not_found_error: None,
            read_action: None,
            identities: vec![Identity::PrimaryKey],
            enable_filter: true,
            enable_sort: true,
            loads: LoadRestrictions::None,
            show_metadata: None,
            metadata_field_names: Vec::new(),
        }
    }
}

impl ActionOptions {
    /// `get?: true`: a read answering one record.
    pub fn get(mut self, get: bool) -> Self {
        self.get = get;
        self
    }

    /// `get_by: fields`: a read answering the one record whose `fields` the request's
    /// `getBy` gives.
    pub fn get_by(mut self, fields: &[&'static str]) -> Self {
        self.get_by = fields.to_vec();
        self
    }

    /// `not_found_error?`: whether a get that finds nothing fails (by default it does)
    /// or answers `null`.
    pub fn not_found_error(mut self, error: bool) -> Self {
        self.not_found_error = Some(error);
        self
    }

    /// `read_action`: the read an update or destroy finds its record through.
    pub fn read_action(mut self, read: &'static str) -> Self {
        self.read_action = Some(read);
        self
    }

    /// `identities`: how an update or destroy may name its record. None: the record the
    /// read finds, with no identity given.
    pub fn identities(mut self, identities: Vec<Identity>) -> Self {
        self.identities = identities;
        self
    }

    /// `enable_filter?`: whether a read takes a filter, its relationships' too.
    pub fn enable_filter(mut self, enable: bool) -> Self {
        self.enable_filter = enable;
        self
    }

    /// `enable_sort?`: whether a read takes a sort, its relationships' too.
    pub fn enable_sort(mut self, enable: bool) -> Self {
        self.enable_sort = enable;
        self
    }

    /// `allowed_loads`: the only loads a request may make (and those on the way to them),
    /// as dotted paths: `comments`, `comments.author`.
    pub fn allowed_loads(mut self, paths: &[&str]) -> Self {
        self.loads = LoadRestrictions::Allow(load_paths(paths));
        self
    }

    /// `denied_loads`: loads a request may not make, nor any under them.
    pub fn denied_loads(mut self, paths: &[&str]) -> Self {
        self.loads = LoadRestrictions::Deny(load_paths(paths));
        self
    }

    /// `show_metadata`: the action metadata a client may ask for (by default, all of
    /// it; none, given none).
    pub fn show_metadata(mut self, fields: &[&'static str]) -> Self {
        self.show_metadata = Some(fields.to_vec());
        self
    }

    /// `metadata_field_names`: the names clients know metadata fields by.
    pub fn metadata_field_names(mut self, names: &[(&'static str, &'static str)]) -> Self {
        self.metadata_field_names = names.to_vec();
        self
    }
}

/// A metadata field a request shows, and the name the client knows it by.
struct Shown {
    def: &'static MetadataDef,
    name: String,
}

/// `record`'s metadata as the request shows it.
fn metadata_json(record: &FieldMap, shown: &[Shown]) -> Map<String, Json> {
    shown
        .iter()
        .map(|field| {
            let value = ash_core::get_metadata(record, field.def.name).map_or(Json::Null, |value| to_json(Some(field.def.ty), value));
            (field.name.clone(), value)
        })
        .collect()
}

fn load_paths(paths: &[&str]) -> Vec<Vec<String>> {
    paths.iter().map(|path| path.split('.').map(str::to_string).collect()).collect()
}

/// An action clients run by name.
struct RpcAction<D> {
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    /// The read an update or destroy finds its record through, when not the default.
    read: Option<&'static ActionDef>,
    options: ActionOptions,
    runner: Option<GenericRunner<D>>,
}

/// The actions an app exposes over RPC, as a domain's `typescript_rpc` block declares
/// them in AshTypescript.
pub struct Rpc<D> {
    actions: HashMap<String, RpcAction<D>>,
    not_found_error: bool,
    on_error: Option<ErrorReporter>,
    error_handler: Option<ErrorHandler>,
    show_raised_errors: bool,
    names: Names,
    /// The actions' names, in the order they were declared.
    declared: Vec<String>,
    typed_queries: Vec<codegen::TypedQuery>,
}

impl<D> Default for Rpc<D> {
    fn default() -> Self {
        Self { actions: HashMap::new(), not_found_error: true, on_error: None, error_handler: None, show_raised_errors: false, names: Names::default(), declared: Vec::new(), typed_queries: Vec::new() }
    }
}

fn action_def(resource: &'static ResourceDef, action: &str) -> &'static ActionDef {
    resource
        .actions
        .iter()
        .find(|a| a.name == action)
        .unwrap_or_else(|| panic!("{} has no action `{action}`", resource.name))
}

impl<D: TransactionSupport + 'static> Rpc<D> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `typed_query :name, :read, fields: [...]`: a read of `R` with its fields chosen up
    /// front, generated into the client as a result type and a fields constant
    /// (`fields` as a request selects them, with snake_case names).
    pub fn typed_query<R: Resource>(mut self, name: &str, action: &str, fields: Json) -> Self {
        let action = action_def(&R::DEF, action);
        assert!(action.kind == ActionKind::Read, "typed query `{name}` reads, and `{}` isn't a read", action.name);
        self.typed_queries.push(codegen::TypedQuery {
            name: name.to_string(),
            resource: &R::DEF,
            action,
            fields,
            result_type_name: None,
            fields_const_name: None,
            description: None,
        });
        self
    }

    fn declare(&mut self, name: &str) {
        if !self.declared.iter().any(|declared| declared == name) {
            self.declared.push(name.to_string());
        }
    }

    /// `field_names`: the names clients know `R`'s fields by, where not their own in
    /// camelCase, as AshTypescript maps them: `("address_line_1", "address_line1")`
    /// names it `addressLine1`.
    pub fn field_names<R: Resource>(mut self, names: &[(&'static str, &'static str)]) -> Self {
        let resource = &R::DEF;
        for (field, _) in names {
            let known = resource.attribute(field).is_some()
                || resource.calculation(field).is_some()
                || resource.aggregate(field).is_some()
                || resource.relationship(field).is_some();
            assert!(known, "{} has no field `{field}` to name", resource.name);
        }
        self.names.map_fields(resource.name, names);
        self
    }

    /// `argument_names`: the names clients know the arguments of `R`'s `action` by.
    pub fn argument_names<R: Resource>(mut self, action: &str, names: &[(&'static str, &'static str)]) -> Self {
        let action = action_def(&R::DEF, action);
        for (argument, _) in names {
            assert!(action.arguments.iter().any(|arg| arg.name == *argument), "`{}` has no argument `{argument}` to name", action.name);
        }
        self.names.map_arguments(R::DEF.name, action.name, names);
        self
    }

    /// `rpc_action :name, :action`: an action of `R`, a generic one run as its own `run`
    /// runs it.
    pub fn action<R: Resource>(self, name: &str, action: &str) -> Self {
        self.action_with::<R>(name, action, |options| options)
    }

    /// `rpc_action :name, :action, options`.
    pub fn action_with<R: Resource>(mut self, name: &str, action: &str, options: impl FnOnce(ActionOptions) -> ActionOptions) -> Self {
        let resource = &R::DEF;
        let action = action_def(resource, action);
        let options = options(ActionOptions::default());
        let read = options.read_action.map(|read| action_def(resource, read));
        for field in &options.get_by {
            assert!(resource.attribute(field).is_some(), "`{name}` gets by `{field}`, which isn't an attribute of {}", resource.name);
        }
        for field in options.show_metadata.iter().flatten().chain(options.metadata_field_names.iter().map(|(field, _)| field)) {
            assert!(action.metadata.iter().any(|def| def.name == *field), "`{name}` shows metadata `{field}`, which `{}` doesn't declare", action.name);
        }
        // A generic action runs as its own `run` does (`Resource::run_generic`).
        let runner: Option<GenericRunner<D>> = (action.kind == ActionKind::Generic).then(|| {
            let name = action.name;
            Arc::new(move |ctx: Context<D>, input: FieldMap| -> BoxFuture<'static, ash_core::Result<Value>> {
                Box::pin(async move { R::run_generic(&ctx, name, input).await })
            }) as GenericRunner<D>
        });
        self.declare(name);
        self.actions.insert(name.into(), RpcAction { resource, action, read, options, runner });
        self
    }

    /// `rpc_action :name, :read, get_by: fields`: one record of `R`, found by `fields`.
    pub fn get_by<R: Resource>(self, name: &str, action: &str, fields: &[&'static str]) -> Self {
        self.action_with::<R>(name, action, |options| options.get_by(fields))
    }

    /// `rpc_action :name, :generic_action`, run by `run` in place of the action's own:
    /// authorized by its policies first as Ash runs one.
    pub fn generic<R: Resource, F, Fut>(mut self, name: &str, action: &str, run: F) -> Self
    where
        F: Fn(Context<D>, FieldMap) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ash_core::Result<Value>> + Send + 'static,
    {
        let def = action_def(&R::DEF, action);
        assert!(def.kind == ActionKind::Generic, "`{name}` isn't a generic action");
        let run = Arc::new(run);
        let runner: GenericRunner<D> = Arc::new(move |ctx: Context<D>, input| {
            let run = Arc::clone(&run);
            Box::pin(async move {
                let inner = ctx.clone();
                ash_core::run::<R, D, Value, _, _>(&ctx, def.name, move || run(inner, input)).await
            })
        });
        self.declare(name);
        self.actions.insert(
            name.into(),
            RpcAction { resource: &R::DEF, action: def, read: None, options: ActionOptions::default(), runner: Some(runner) },
        );
        self
    }

    /// AshTypescript's `not_found_error?` default for gets: whether one that finds
    /// nothing fails (it does, by default) or answers `null`.
    pub fn not_found_error(mut self, error: bool) -> Self {
        self.not_found_error = error;
        self
    }

    /// Tells `report` of each error AshTypescript has no shape for, which the client
    /// sees only as an internal error with an id, as AshTypescript logs one.
    pub fn on_error(mut self, report: impl Fn(&str, &Error) + Send + Sync + 'static) -> Self {
        self.on_error = Some(Arc::new(report));
        self
    }

    /// Rewrites each error before the client sees it, or drops it, as AshTypescript's
    /// domain `error_handler` does.
    pub fn error_handler(mut self, handle: impl Fn(Failure, &ErrorSource) -> Option<Failure> + Send + Sync + 'static) -> Self {
        self.error_handler = Some(Arc::new(handle));
        self
    }

    /// AshTypescript's `show_raised_errors?`: an error it has no shape for shows what it
    /// says, not only its id.
    pub fn show_raised_errors(mut self, show: bool) -> Self {
        self.show_raised_errors = show;
        self
    }

    /// Runs `request` in `ctx`, its `tenant` overriding the context's, and answers as
    /// AshTypescript does.
    pub async fn run(&self, ctx: &Context<D>, request: &Json) -> Json {
        match self.answer(ctx, request).await {
            Ok(Answer { data, metadata: None }) => json!({ "success": true, "data": data }),
            // A write's metadata beside its data, as AshTypescript answers it.
            Ok(Answer { data, metadata: Some(metadata) }) => json!({ "success": true, "data": data, "metadata": metadata }),
            Err(failures) => json!({ "success": false, "errors": self.report(failures, request) }),
        }
    }

    /// `failures`, each with those reported with it, as the client sees them: through
    /// the error handler.
    fn report(&self, failures: Vec<Failure>, request: &Json) -> Vec<Json> {
        let name = request["action"].as_str();
        let rpc = name.and_then(|name| self.actions.get(name));
        let source = ErrorSource {
            rpc_action: name.map(str::to_string),
            resource: rpc.map(|rpc| rpc.resource.name),
            action: rpc.map(|rpc| rpc.action.name),
        };
        failures
            .into_iter()
            .flat_map(Failure::flatten)
            .filter_map(|failure| match &self.error_handler {
                Some(handle) => handle(failure, &source),
                None => Some(failure),
            })
            .map(|failure| failure.to_json())
            .collect()
    }

    /// An error of running `action`, as AshTypescript reports it: a missing field named
    /// as an attribute or an argument of it.
    fn failure_of(&self, action: &ActionDef, err: Error) -> Failure {
        let argument = matches!(&err, Error::Missing { field } if action.arguments.iter().any(|arg| arg.name == field));
        self.failure(err).required_as(argument)
    }

    /// An ash-core error as AshTypescript reports it; one it has no shape for, as an
    /// internal error the server is told of.
    fn failure(&self, err: Error) -> Failure {
        if let Error::Multiple(errors) = err {
            let mut failures = errors.into_iter().map(|error| self.failure(error));
            let mut first = failures.next().unwrap_or_else(|| self.failure(Error::Invalid("no errors".into())));
            first.rest.extend(failures);
            return first;
        }
        Failure::from_error(&err).unwrap_or_else(|| {
            let id = uuid::Uuid::new_v4().to_string();
            if let Some(report) = &self.on_error {
                report(&id, &err);
            }
            let mut failure = Failure::internal(id);
            if self.show_raised_errors {
                failure.message = err.to_string();
            }
            failure
        })
    }

    /// Validates `request` without running it, as AshTypescript's `validate_action` does
    /// (`POST /rpc/validate`): checked as a run would check it, then its input as a form
    /// validates one, every problem reported against its field. An update or destroy's
    /// record is found first. Nothing is written, nor authorized.
    pub async fn validate(&self, ctx: &Context<D>, request: &Json) -> Json {
        match self.validation(ctx, request).await {
            Ok(()) => json!({ "success": true }),
            Err(failures) => json!({ "success": false, "errors": self.report(failures, request) }),
        }
    }

    async fn validation(&self, ctx: &Context<D>, request: &Json) -> Result<(), Vec<Failure>> {
        let Parsed { rpc, ctx, input, .. } = self.parse(ctx, request, true).map_err(|failure| vec![failure])?;
        let (resource, action) = (rpc.resource, rpc.action);
        // Each value cast to its field's type, as a form casts it.
        let Json::Object(given) = &input else { return Ok(()) };
        let mut problems = Vec::new();
        let mut cast = FieldMap::new();
        for (name, value) in given {
            let ty = action
                .arguments
                .iter()
                .find(|arg| arg.name == name)
                .map(|arg| arg.ty)
                .or_else(|| action.accept.contains(&name.as_str()).then(|| resource.attribute(name).map(|attr| attr.ty)).flatten());
            let Some(ty) = ty else { continue };
            match value_input(ty, value) {
                Ok(value) => {
                    cast.insert(name.clone(), value);
                }
                Err(_) => problems.push(Error::TypeMismatch { field: name.clone(), expected: ty.name().to_string(), got: value.to_string() }),
            }
        }
        let invalid: Vec<String> = problems.iter().filter_map(|p| match p { Error::TypeMismatch { field, .. } => Some(field.clone()), _ => None }).collect();
        let existing = match action.kind {
            ActionKind::Update | ActionKind::Destroy => {
                let id = self.identity(&ctx, rpc, request.get("identity")).await.map_err(|failure| vec![failure])?;
                Some(self.found(&ctx, rpc, id).await.map_err(|failure| vec![failure])?)
            }
            _ => None,
        };
        if !matches!(action.kind, ActionKind::Create | ActionKind::Update | ActionKind::Destroy) {
            ash_core::apply_argument_defaults(action.arguments, &mut cast);
        }
        match action.kind {
            ActionKind::Create | ActionKind::Update | ActionKind::Destroy => {
                problems.extend(ash_core::DynamicChangeset::problems(&ctx, resource, action, existing, cast));
            }
            // A read or generic action: its arguments, each it requires given or defaulted.
            _ => problems.extend(
                action
                    .arguments
                    .iter()
                    .filter(|arg| !arg.allow_nil && cast.get(arg.name).is_none_or(Value::is_null) && !invalid.iter().any(|field| field == arg.name))
                    .map(|arg| Error::Missing { field: arg.name.to_string() }),
            ),
        }
        // A value that isn't of its type is invalid, not missing too.
        problems.retain(|problem| !matches!(problem, Error::Missing { field } if invalid.contains(field)));
        if problems.is_empty() {
            return Ok(());
        }
        Err(problems.into_iter().map(|problem| self.form_failure(resource, action, problem)).collect())
    }

    /// A problem a form finds, as AshTypescript reports it: an invalid attribute, its
    /// message filled in, at its field.
    fn form_failure(&self, resource: &ResourceDef, action: &ActionDef, problem: Error) -> Failure {
        let (field, message) = match &problem {
            Error::Validation { field, .. } => (field.clone(), problem.message()),
            Error::Missing { field } => (field.clone(), "is required".to_string()),
            Error::TypeMismatch { field, .. } => (field.clone(), "is invalid".to_string()),
            _ => return self.failure(problem),
        };
        let name = if action.arguments.iter().any(|arg| arg.name == field) {
            self.names.argument(resource, action.name, &field)
        } else {
            self.names.field(resource, &field)
        };
        Failure::form_error(&name, message)
    }

    /// Record `id`, as the read the action finds records through finds it.
    async fn found(&self, ctx: &Context<D>, rpc: &RpcAction<D>, id: Value) -> Result<FieldMap, Failure> {
        let resource = rpc.resource;
        let pk = resource.primary_key().map(|attr| attr.name).unwrap_or("id");
        let read = rpc.read.or_else(|| resource.primary_read()).ok_or_else(|| self.failure(Error::NoPrimaryRead(resource.name)))?;
        let query = CompiledQuery { filter: Some(Filter::eq(pk, id.clone())), tenant: ctx.tenant.clone(), limit: Some(1), ..CompiledQuery::default() };
        let scoped = ash_core::scope_read(resource, read, ctx.actor.as_ref(), &FieldMap::new(), query).map_err(|e| self.failure(e))?;
        let rows = ctx.data.run_query(resource, &scoped).await.map_err(|e| self.failure(e))?;
        rows.into_iter().next().ok_or_else(|| Failure::new("not_found", "Not found", format!("record with id: {:?} not found", id.to_string())))
    }

    async fn answer(&self, ctx: &Context<D>, request: &Json) -> Result<Answer, Vec<Failure>> {
        self.answer_one(ctx, request).await.map_err(|failure| vec![failure])
    }

    /// `request` checked as AshTypescript checks one before running it: the action it
    /// names, what that action needs and can't take, what it selects, its input's shape,
    /// a get's `getBy` and a read's page. Validating it, a read needn't select anything.
    fn parse<'a>(&'a self, ctx: &Context<D>, request: &Json, validating: bool) -> Result<Parsed<'a, D>, Failure> {
        let name = request["action"].as_str().unwrap_or_default();
        let rpc = self.actions.get(name).ok_or_else(|| Failure::action_not_found(name))?;
        let (resource, action, options) = (rpc.resource, rpc.action, &rpc.options);
        let mut ctx = ctx.clone();
        if let Some(tenant) = request["tenant"].as_str() {
            ctx = ctx.with_tenant(tenant);
        }
        let given = |key: &str| request.get(key).filter(|value| !value.is_null());

        // What the action needs, and what it can't take.
        let read = action.kind == ActionKind::Read;
        let get = read && (options.get || !options.get_by.is_empty());
        if read && !validating {
            match request.get("fields") {
                None | Some(Json::Null) => return Err(Failure::missing_required_parameter("fields")),
                Some(Json::Array(items)) if items.is_empty() => return Err(Failure::empty_fields_array()),
                Some(Json::Array(_)) => {}
                Some(other) => return Err(Failure::invalid_fields_type(other)),
            }
        }
        let list = read && !get;
        for (option, enabled) in [("filter", options.enable_filter), ("sort", options.enable_sort)] {
            if given(option).is_some() {
                if !list {
                    return Err(Failure::option_not_supported(option, "unsupported", None));
                }
                if !enabled {
                    return Err(Failure::option_not_supported(option, "disabled", None));
                }
            }
        }
        if given("page").is_some() && !(list && action.pagination.is_some()) {
            return Err(Failure::option_not_supported("page", "unsupported", None));
        }

        // What it selects.
        let rules = Rules { actor: ctx.actor.as_ref(), enable_filter: options.enable_filter, enable_sort: options.enable_sort, loads: &options.loads, names: &self.names };
        let selection = match given("fields") {
            None => Selection::default(),
            Some(Json::Array(items)) if validating && items.is_empty() => Selection::default(),
            Some(fields) => Selection::parse(resource, fields, &[], &rules)?,
        };

        // Its input, the record a get finds, and its page.
        let raw_input = match request.get("input") {
            None => Json::Object(Map::new()),
            Some(input @ Json::Object(_)) => input.clone(),
            Some(other) => return Err(Failure::invalid_input_format(other)),
        };
        let get_by = if options.get_by.is_empty() { Vec::new() } else { get_by_filters(&self.names, resource, &options.get_by, request.get("getBy"))? };
        let page = match given("page") {
            None => None,
            Some(page) => Some(PageRequest::parse(page)?),
        };
        let shown = shown_metadata(rpc, request.get("metadataFields"));
        Ok(Parsed { rpc, ctx, get, selection, input: self.names.input(resource, action, &raw_input), get_by, page, shown })
    }

    async fn answer_one(&self, ctx: &Context<D>, request: &Json) -> Result<Answer, Failure> {
        let Parsed { rpc, ctx, get, selection, input: raw_input, get_by, page, shown } = self.parse(ctx, request, false)?;
        let (resource, action, options) = (rpc.resource, rpc.action, &rpc.options);
        let name = request["action"].as_str().unwrap_or_default();
        let input = action_input(resource, action, &raw_input).map_err(|e| self.failure_of(action, e))?;
        // What a write notes on its record, as the request shows it.
        let metadata = |stored: &FieldMap| (!shown.is_empty()).then(|| metadata_json(stored, &shown));

        match action.kind {
            ActionKind::Read if get => {
                let found = self.get(&ctx, rpc, &selection, input, get_by, &shown).await?;
                Ok(match found {
                    Some(record) => record,
                    None if options.not_found_error.unwrap_or(self.not_found_error) => return Err(self.failure(Error::NotFound)),
                    None => Json::Null,
                }
                .into())
            }
            ActionKind::Read => self.list(&ctx, rpc, &selection, input, request, page, &shown).await.map(Answer::from),
            ActionKind::Create => {
                let stored = ash_core::create_dynamic(&ctx, resource, action, input).await.map_err(|e| self.failure_of(action, e))?;
                let metadata = metadata(&stored);
                Ok(Answer { data: self.written(&ctx, resource, stored, &selection).await?, metadata })
            }
            ActionKind::Update => {
                let id = self.identity(&ctx, rpc, request.get("identity")).await?;
                // Not found before it runs, or failing as it runs, for its one record.
                let stored = ash_core::update_dynamic_via(&ctx, resource, rpc.read, action, id, input, None)
                    .await
                    .map_err(|e| match e {
                        Error::NotFound => self.failure(e),
                        e => self.failure_of(action, e).in_bulk(),
                    })?;
                let metadata = metadata(&stored);
                Ok(Answer { data: self.written(&ctx, resource, stored, &selection).await?, metadata })
            }
            ActionKind::Destroy => {
                // Nothing to destroy: a bulk destroy of no records, which succeeds, its
                // selected fields as an empty record's.
                let nothing = || Json::Object(
                    selection.fields.iter().chain(selection.relationships.iter().map(|(name, _)| name)).map(|name| (self.names.field(resource, name), Json::Null)).collect(),
                );
                let id = match self.identity(&ctx, rpc, request.get("identity")).await {
                    Err(failure) if failure.kind == "not_found" => return Ok(nothing().into()),
                    found => found?,
                };
                // What it selects of the record, read before it goes.
                let loaded = self.loads_of(&ctx, resource, id.clone(), &selection).await?;
                match ash_core::destroy_dynamic_via(&ctx, resource, rpc.read, action, id, input, None).await {
                    Err(Error::NotFound) => Ok(nothing().into()),
                    Err(e) => Err(self.failure_of(action, e)),
                    Ok(mut stored) => {
                        let metadata = metadata(&stored);
                        stored.extend(loaded);
                        let rows = self.render(&ctx, resource, vec![stored], &selection).await?;
                        Ok(Answer { data: rows.into_iter().next().unwrap_or(Json::Null), metadata })
                    }
                }
            }
            ActionKind::Generic => {
                let runner = rpc.runner.as_ref().ok_or_else(|| Failure::action_not_found(name))?;
                let value = runner(ctx, input).await.map_err(|e| self.failure_of(action, e))?;
                // An action returning nothing answers an empty object, as AshTypescript's
                // (`:ok`) does.
                Ok(if action.returns.is_none() { json!({}) } else { to_json(action.returns, &value) }.into())
            }
        }
    }

    /// The record an update or destroy names by `identity`: its primary key's value, or
    /// the fields of one of the action's identities, found through the read the action
    /// finds records through. An action taking no identity changes the one record that
    /// read finds.
    async fn identity(&self, ctx: &Context<D>, rpc: &RpcAction<D>, given: Option<&Json>) -> Result<Value, Failure> {
        let resource = rpc.resource;
        let identities = &rpc.options.identities;
        let pk = resource.primary_key().map(|attr| attr.name).unwrap_or("id");
        let expected = || -> Vec<String> {
            identities
                .iter()
                .flat_map(|identity| match identity {
                    Identity::PrimaryKey => vec![to_camel_case(pk)],
                    Identity::Named(name) => resource
                        .identities
                        .iter()
                        .find(|i| i.name == *name)
                        .map(|i| i.keys.iter().map(|key| to_camel_case(key)).collect())
                        .unwrap_or_default(),
                })
                .collect()
        };
        let filter = match given.filter(|value| !value.is_null()) {
            None if identities.is_empty() => Filter::True,
            None => return Err(Failure::missing_identity(expected(), identities == &[Identity::PrimaryKey])),
            // A primary key given directly: the record by id, with no read first.
            Some(given @ (Json::String(_) | Json::Number(_))) if identities.contains(&Identity::PrimaryKey) => {
                let ty = resource.primary_key().map_or(AttrType::Uuid, |attr| attr.ty);
                let invalid = || self.failure(Error::Invalid(format!("invalid primary key {given}")));
                let value = value_input(ty, given).map_err(|_| invalid())?;
                return ash_core::pk_cast(ty, &value).ok_or_else(invalid);
            }
            Some(Json::Object(fields)) => {
                let fields: HashMap<String, &Json> = fields.iter().map(|(key, value)| (self.names.field_named(resource, key), value)).collect();
                let matched = identities.iter().find_map(|identity| {
                    let keys: Vec<&str> = match identity {
                        // A map names a composite primary key; a single one is given
                        // directly, as AshTypescript takes them.
                        Identity::PrimaryKey => {
                            let keys: Vec<&str> = resource.attributes.iter().filter(|attr| attr.primary_key).map(|attr| attr.name).collect();
                            if keys.len() < 2 {
                                return None;
                            }
                            keys
                        }
                        Identity::Named(name) => resource.identities.iter().find(|i| i.name == *name)?.keys.to_vec(),
                    };
                    keys.iter().all(|key| fields.contains_key(*key)).then_some(keys)
                });
                let Some(keys) = matched else {
                    let provided = fields.keys().map(|key| to_camel_case(key)).collect();
                    return Err(Failure::invalid_identity(provided, expected()));
                };
                let mut parts = Vec::new();
                for key in keys {
                    let value = fields[key];
                    if value.is_object() || value.is_array() {
                        return Err(Failure::invalid_identity_value(format!(
                            "Identity values must be scalar equality operands. Non-scalar value provided for: {}",
                            to_camel_case(key)
                        )));
                    }
                    let ty = field_type(resource, key).unwrap_or(AttrType::String);
                    parts.push(Filter::eq(key, value_input(ty, value).map_err(|e| self.failure(e))?));
                }
                Filter::and(parts)
            }
            Some(other) => return Err(Failure::invalid_identity(vec![other.to_string()], expected())),
        };
        // Found through the read, as the actor may read it: its id.
        let read = rpc.read.or_else(|| resource.primary_read()).ok_or_else(|| self.failure(Error::NoPrimaryRead(resource.name)))?;
        let query = CompiledQuery { filter: Some(filter), tenant: ctx.tenant.clone(), limit: Some(1), select: Some(vec![pk.to_string()]), ..CompiledQuery::default() };
        let scoped = ash_core::scope_read(resource, read, ctx.actor.as_ref(), &FieldMap::new(), query).map_err(|e| self.failure(e))?;
        let rows = ctx.data.run_query(resource, &scoped).await.map_err(|e| self.failure(e))?;
        rows.first().and_then(|row| row.get(pk)).filter(|id| !id.is_null()).cloned().ok_or_else(|| self.failure(Error::NotFound))
    }

    /// The one record a get finds, as selected.
    async fn get(&self, ctx: &Context<D>, rpc: &RpcAction<D>, selection: &Selection, input: FieldMap, get_by: Vec<Filter>, shown: &[Shown]) -> Result<Option<Json>, Failure> {
        let resource = rpc.resource;
        let base = CompiledQuery { filter: Some(Filter::and(get_by)), sort: prepared_sort(rpc.action), tenant: ctx.tenant.clone(), ..CompiledQuery::default() };
        let scoped = ash_core::scope_read(resource, rpc.action, ctx.actor.as_ref(), &input, base).map_err(|e| self.failure(e))?;
        let rows = read_rows(ctx, resource, selection, CompiledQuery { limit: Some(2), ..scoped }).await.map_err(|e| self.failure(e))?;
        match rows.len() {
            0 => Ok(None),
            1 => Ok(self.render_read(ctx, rpc, &input, rows, selection, shown).await?.into_iter().next()),
            n => Err(self.failure(Error::TooMany(n))),
        }
    }

    /// A list read: its records, or a page of them, as its action's pagination decides.
    #[allow(clippy::too_many_arguments)]
    async fn list(&self, ctx: &Context<D>, rpc: &RpcAction<D>, selection: &Selection, input: FieldMap, request: &Json, page: Option<PageRequest>, shown: &[Shown]) -> Result<Json, Failure> {
        let (resource, action) = (rpc.resource, rpc.action);
        let actor = ctx.actor.as_ref();
        let mut filters = Vec::new();
        if let Some(filter) = request.get("filter").filter(|value| !value.is_null()) {
            let filter = filter_input(resource, &self.names.filter(resource, filter)).map_err(|e| self.failure(e).under("filter"))?;
            filters.push(ash_core::guard_input_filter(resource, actor, filter).map_err(|e| self.failure(e))?);
        }
        // The action's sort, then the client's, as AshTypescript sorts its query after
        // running the read's preparations.
        let mut sort = prepared_sort(action);
        if let Some(text) = request.get("sort").filter(|value| !value.is_null()).map(sort_text) {
            let given = sort_input(resource, &self.names.sort(resource, &text)).map_err(|e| self.failure(e).under("sort"))?;
            sort.extend(ash_core::guard_input_sort(resource, actor, given).map_err(|e| self.failure(e))?);
        }
        let base = CompiledQuery { filter: Some(Filter::and(filters)), sort, tenant: ctx.tenant.clone(), ..CompiledQuery::default() };
        let scoped = ash_core::scope_read(resource, action, actor, &input, base).map_err(|e| self.failure(e))?;

        let Some(pagination) = action.pagination else {
            let rows = read_rows(ctx, resource, selection, scoped).await.map_err(|e| self.failure(e))?;
            return Ok(Json::Array(self.render_read(ctx, rpc, &input, rows, selection, shown).await?));
        };
        // As Ash pages a read: the default limit where none is given, and where the read
        // must page, a page of it even unasked.
        let mut page = page;
        if let Some(page) = &mut page
            && page.limit.is_none()
        {
            page.limit = pagination.default_limit;
        }
        if page.is_none() && pagination.required {
            page = pagination.default_limit.map(|limit| PageRequest { limit: Some(limit), ..PageRequest::default() });
        }
        if page.as_ref().is_some_and(|page| page.count == Some(true)) && pagination.countable == Countable::No {
            return Err(Failure::new("invalid_page", "Invalid pagination", format!("Action {} cannot be counted", action.name)));
        }
        let Some((page, limit)) = page.and_then(|page| page.limit.map(|limit| (page, limit))) else {
            if pagination.required {
                return Err(Failure::new("invalid_page", "Invalid pagination", "Limit is required"));
            }
            let rows = read_rows(ctx, resource, selection, scoped).await.map_err(|e| self.failure(e))?;
            return Ok(Json::Array(self.render_read(ctx, rpc, &input, rows, selection, shown).await?));
        };
        // The smallest of the page's size, the read's own limit and its largest page, as
        // Ash takes it.
        let limit = [Some(limit), scoped.limit, pagination.max_page_size].into_iter().flatten().min().unwrap_or(limit);
        let count = if page.count == Some(true) || (pagination.countable == Countable::ByDefault && page.count != Some(false)) {
            let counted = CompiledQuery { sort: Vec::new(), ..scoped.clone() };
            Some(ctx.data.count(resource, &counted).await.map_err(|e| self.failure(e))?)
        } else {
            None
        };
        let keyset = page.after.is_some() || page.before.is_some() || (page.offset.is_none() && pagination.keyset);
        if !keyset {
            let offset = page.offset.unwrap_or(0);
            let mut rows = read_rows(ctx, resource, selection, CompiledQuery { limit: Some(limit + 1), offset: Some(offset), ..scoped })
                .await
                .map_err(|e| self.failure(e))?;
            let has_more = rows.len() > limit;
            rows.truncate(limit);
            let results = self.render_read(ctx, rpc, &input, rows, selection, shown).await?;
            return Ok(json!({
                "results": results, "hasMore": has_more, "limit": limit, "offset": offset, "count": count, "type": "offset",
            }));
        }
        let page = self.keyset_page(ctx, rpc, &input, selection, scoped, limit, &page, shown).await?;
        Ok(json!({
            "results": page.results, "hasMore": page.has_more, "limit": limit, "after": page.after, "before": page.before,
            "nextPage": page.next, "previousPage": page.previous, "count": count, "type": "keyset",
        }))
    }

    /// A keyset page of what `scoped` reads: after (or before) its cursor, in a stable
    /// sort.
    #[allow(clippy::too_many_arguments)]
    async fn keyset_page(
        &self,
        ctx: &Context<D>,
        rpc: &RpcAction<D>,
        input: &FieldMap,
        selection: &Selection,
        scoped: CompiledQuery,
        limit: usize,
        page: &PageRequest,
        shown: &[Shown],
    ) -> Result<KeysetPage, Failure> {
        let resource = rpc.resource;
        let sort = ash_core::keyset_sort(resource, scoped.sort.clone());
        let pk = resource.primary_key().map(|attr| attr.name).unwrap_or("id");
        // Ash reads before a keyset when given both.
        let backward = page.before.is_some();
        let mut filter = scoped.filter.clone();
        if let Some(cursor) = page.before.as_deref().or(page.after.as_deref()) {
            let direction = if backward { "before" } else { "after" };
            let keyset = KeysetCursor::decode(cursor)
                .filter(|keyset| keyset.values.len() == sort.len() && keyset.values.iter().zip(&sort).all(|((field, _), s)| *field == s.field))
                .ok_or_else(|| Failure::invalid_keyset(cursor, direction))?;
            let values = ash_core::keyset_values(resource, &keyset, &sort);
            if let Some(after) = ash_core::build_keyset_filter(resource, &sort, &values, !backward) {
                filter = Some(Filter::and(filter.into_iter().chain([after])));
            }
        }
        let read_sort: Vec<Sort> = sort.iter().map(|s| if backward { s.reversed() } else { s.clone() }).collect();
        let query = CompiledQuery { filter, sort: read_sort, limit: Some(limit + 1), offset: None, ..scoped };
        let mut rows = read_rows(ctx, resource, &selection.keeping(sort.iter().map(|s| s.field.as_str())), query)
            .await
            .map_err(|e| self.failure(e))?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        if backward {
            rows.reverse();
        }
        let cursor = |row: Option<&FieldMap>| -> Json {
            row.map_or(Json::Null, |row| {
                let id = row.get(pk).cloned().unwrap_or(Value::Null);
                let values = sort.iter().map(|s| (s.field.clone(), row.get(&s.field).cloned().unwrap_or(Value::Null))).collect();
                Json::String(KeysetCursor { id, values }.encode())
            })
        };
        let (previous, next) = (cursor(rows.first()), cursor(rows.last()));
        let results = self.render_read(ctx, rpc, input, rows, selection, shown).await?;
        Ok(KeysetPage { results, has_more, after: page.after.clone(), before: page.before.clone(), next, previous })
    }

    /// The aggregates and calculations `selection` loads of record `id`, as the actor
    /// reads them.
    async fn loads_of(&self, ctx: &Context<D>, resource: &'static ResourceDef, id: Value, selection: &Selection) -> Result<FieldMap, Failure> {
        let loads = selection.of(|name| resource.attribute(name).is_none());
        let Some(pk) = resource.primary_key().filter(|_| !loads.is_empty()) else {
            return Ok(FieldMap::new());
        };
        let query = CompiledQuery {
            filter: Some(Filter::eq(pk.name, id)),
            tenant: ctx.tenant.clone(),
            actor: ctx.actor.clone(),
            calculation_args: selection.calculation_args.clone(),
            ..CompiledQuery::default()
        };
        let row = read_rows(ctx, resource, selection, query).await.map_err(|e| self.failure(e))?.into_iter().next().unwrap_or_default();
        Ok(loads.into_iter().filter_map(|name| row.get(&name).cloned().map(|value| (name, value))).collect())
    }

    /// A written record, as the client selected it: its aggregates and calculations read as
    /// the actor sees them, and its relationships loaded.
    async fn written(&self, ctx: &Context<D>, resource: &'static ResourceDef, mut stored: FieldMap, selection: &Selection) -> Result<Json, Failure> {
        if let Some(id) = resource.primary_key().and_then(|pk| stored.get(pk.name)).filter(|id| !id.is_null()).cloned() {
            stored.extend(self.loads_of(ctx, resource, id, selection).await?);
        }
        let rows = self.render(ctx, resource, vec![stored], selection).await?;
        Ok(rows.into_iter().next().unwrap_or(Json::Null))
    }

    /// The records a read through `rpc`'s action found, after its `after_action`
    /// preparations, as the client selected them, with the metadata the request shows
    /// on each, as AshTypescript merges a read's metadata into its records.
    async fn render_read(
        &self,
        ctx: &Context<D>,
        rpc: &RpcAction<D>,
        input: &FieldMap,
        mut rows: Vec<FieldMap>,
        selection: &Selection,
        shown: &[Shown],
    ) -> Result<Vec<Json>, Failure> {
        ash_core::after_read(rpc.action, input, &mut rows).map_err(|e| self.failure(e))?;
        let metadata: Vec<Map<String, Json>> = rows.iter().map(|row| metadata_json(row, shown)).collect();
        let mut rendered = self.render(ctx, rpc.resource, rows, selection).await?;
        for (record, metadata) in rendered.iter_mut().zip(metadata) {
            if let Json::Object(record) = record {
                record.extend(metadata);
            }
        }
        Ok(rendered)
    }

    /// `rows` as the client selected them, their relationships loaded in turn.
    fn render<'a>(&'a self, ctx: &'a Context<D>, resource: &'static ResourceDef, rows: Vec<FieldMap>, selection: &'a Selection) -> BoxFuture<'a, Result<Vec<Json>, Failure>> {
        Box::pin(async move {
            let mut out: Vec<Map<String, Json>> = rows
                .iter()
                .map(|row| {
                    selection
                        .fields
                        .iter()
                        .map(|name| {
                            let value = row.get(name).map_or(Json::Null, |value| match selection.values.get(name) {
                                Some(within) => select_value(field_type(resource, name), value, within),
                                None => to_json(field_type(resource, name), value),
                            });
                            (self.names.field(resource, name), value)
                        })
                        .collect()
                })
                .collect();
            for (rel_name, nested) in &selection.relationships {
                let rel = resource.relationship(rel_name).expect("parsed against the resource");
                let dest = (rel.destination)();
                let many = matches!(rel.kind, RelKind::HasMany | RelKind::ManyToMany);
                let values: Vec<Json> = match &nested.page {
                    None => {
                        let groups = ash_core::load_related_query(ctx, resource, rel_name, &rows, &nested.query).await.map_err(|e| self.failure(e))?;
                        let sizes: Vec<usize> = groups.iter().map(Vec::len).collect();
                        let mut rendered = self.render(ctx, dest, groups.into_iter().flatten().collect(), &nested.selection).await?.into_iter();
                        sizes
                            .into_iter()
                            .map(|size| {
                                let related: Vec<Json> = rendered.by_ref().take(size).collect();
                                if many { Json::Array(related) } else { related.into_iter().next().unwrap_or(Json::Null) }
                            })
                            .collect()
                    }
                    Some(page) => self.nested_pages(ctx, resource, rel_name, dest, &rows, nested, page).await?,
                };
                for (object, value) in out.iter_mut().zip(values) {
                    object.insert(self.names.field(resource, rel_name), value);
                }
            }
            Ok(out.into_iter().map(Json::Object).collect())
        })
    }

    /// A page of a relationship's rows for each of `rows`, as AshTypescript shapes one.
    #[allow(clippy::too_many_arguments)]
    async fn nested_pages(
        &self,
        ctx: &Context<D>,
        resource: &'static ResourceDef,
        rel_name: &str,
        dest: &'static ResourceDef,
        rows: &[FieldMap],
        nested: &fields::Nested,
        page: &NestedPage,
    ) -> Result<Vec<Json>, Failure> {
        let pk = dest.primary_key().map(|attr| attr.name).unwrap_or("id");
        let mut query: RelatedQuery = nested.query.clone();
        // The request's sort, then the read's prepared sort, then the primary key to break
        // ties, as Ash adds its stability sort to the prepared query.
        let mut sort = query.sort.clone();
        for prepared in prepared_sort(dest.default_read()) {
            if !sort.iter().any(|given| given.field == prepared.field) {
                sort.push(prepared);
            }
        }
        let mut sort = ash_core::keyset_sort(dest, sort);
        let backward = page.keyset && page.before.is_some();
        if page.keyset {
            if let Some(cursor) = page.before.as_deref().or(page.after.as_deref()) {
                let direction = if backward { "before" } else { "after" };
                let keyset = KeysetCursor::decode(cursor)
                    .filter(|keyset| keyset.values.len() == sort.len() && keyset.values.iter().zip(&sort).all(|((field, _), s)| field == &s.field))
                    .ok_or_else(|| Failure::invalid_keyset(cursor, direction))?;
                let values = ash_core::keyset_values(dest, &keyset, &sort);
                if let Some(after) = ash_core::build_keyset_filter(dest, &sort, &values, !backward) {
                    query.filter = Some(Filter::and(query.filter.take().into_iter().chain([after])));
                }
            }
            if backward {
                sort = sort.iter().map(Sort::reversed).collect();
            }
            query.sort = sort.clone();
            // The cursor holds every sort field, a calculation or aggregate included, so
            // the next page filters on the value the row sorted by.
            keep_sort_fields(dest, &mut query, &sort);
        } else {
            query.offset = page.offset;
        }
        let limit = page.limit;
        query.limit = limit.map(|limit| limit + 1);
        let groups = ash_core::load_related_query(ctx, resource, rel_name, rows, &query).await.map_err(|e| self.failure(e))?;
        let counts = if page.count {
            let action = dest.default_read();
            if action.pagination.is_some_and(|pagination| pagination.countable == Countable::No) {
                return Err(Failure::new("invalid_page", "Invalid pagination", format!("Action {} cannot be counted", action.name)));
            }
            let counting =
                RelatedQuery { filter: nested.query.filter.clone(), calculation_args: nested.query.calculation_args.clone(), ..RelatedQuery::default() };
            ash_core::count_related_query(ctx, resource, rel_name, rows, &counting).await.map_err(|e| self.failure(e))?.into_iter().map(Json::from).collect()
        } else {
            vec![Json::Null; rows.len()]
        };
        let mut out = Vec::with_capacity(groups.len());
        for (mut group, count) in groups.into_iter().zip(counts) {
            let has_more = limit.is_some_and(|limit| group.len() > limit);
            if let Some(limit) = limit {
                group.truncate(limit);
            }
            if backward {
                group.reverse();
            }
            let cursor = |row: Option<&FieldMap>| -> Json {
                row.map_or(Json::Null, |row| {
                    let id = row.get(pk).cloned().unwrap_or(Value::Null);
                    let values = sort.iter().map(|s| (s.field.clone(), row.get(&s.field).cloned().unwrap_or(Value::Null))).collect();
                    Json::String(KeysetCursor { id, values }.encode())
                })
            };
            let (previous, next) = (cursor(group.first()), cursor(group.last()));
            let results = self.render(ctx, dest, group, &nested.selection).await?;
            out.push(if page.keyset {
                json!({
                    "results": results, "hasMore": has_more, "limit": limit, "after": page.after, "before": page.before,
                    "nextPage": next, "previousPage": previous, "count": count, "type": "keyset",
                })
            } else {
                json!({ "results": results, "hasMore": has_more, "limit": limit, "offset": page.offset.unwrap_or(0), "count": count, "type": "offset" })
            });
        }
        Ok(out)
    }
}

/// A request, checked: the action it runs, in the context it runs in, what it selects,
/// its input with snake_case names, a get's filters and a read's page.
struct Parsed<'a, D> {
    rpc: &'a RpcAction<D>,
    ctx: Context<D>,
    get: bool,
    selection: Selection,
    input: Json,
    get_by: Vec<Filter>,
    page: Option<PageRequest>,
    /// The action metadata the request shows.
    shown: Vec<Shown>,
}

/// What a request answers: its data, and for a write, the metadata it shows.
struct Answer {
    data: Json,
    metadata: Option<Map<String, Json>>,
}

impl From<Json> for Answer {
    fn from(data: Json) -> Self {
        Self { data, metadata: None }
    }
}

/// The action metadata a request shows, as AshTypescript picks it: what its
/// `metadataFields` names (by the client's name for it, or its own) of what the action
/// exposes, or given none, all it exposes for a write and none for a read.
fn shown_metadata<D>(rpc: &RpcAction<D>, requested: Option<&Json>) -> Vec<Shown> {
    let options = &rpc.options;
    let exposed: Vec<&'static MetadataDef> = rpc
        .action
        .metadata
        .iter()
        .filter(|def| options.show_metadata.as_ref().is_none_or(|shown| shown.contains(&def.name)))
        .collect();
    let name_of = |def: &MetadataDef| {
        options
            .metadata_field_names
            .iter()
            .find(|(field, _)| *field == def.name)
            .map_or_else(|| to_camel_case(def.name), |(_, name)| name.to_string())
    };
    let requested: Vec<&str> = requested.and_then(Json::as_array).map(|items| items.iter().filter_map(Json::as_str).collect()).unwrap_or_default();
    let mut picked: Vec<&'static MetadataDef> = Vec::new();
    if requested.is_empty() {
        if matches!(rpc.action.kind, ActionKind::Create | ActionKind::Update | ActionKind::Destroy) {
            picked = exposed;
        }
    } else {
        for given in requested {
            let found = exposed.iter().copied().find(|def| name_of(def) == given || def.name == snake(given));
            if let Some(def) = found.filter(|def| !picked.iter().any(|seen| seen.name == def.name)) {
                picked.push(def);
            }
        }
    }
    picked.into_iter().map(|def| Shown { def, name: name_of(def) }).collect()
}

struct KeysetPage {
    results: Vec<Json>,
    has_more: bool,
    after: Option<String>,
    before: Option<String>,
    next: Json,
    previous: Json,
}

/// A request's `page`, as AshTypescript takes it: a map of `limit`, `offset`, `count`,
/// `after` and `before`.
#[derive(Clone, Debug, Default)]
struct PageRequest {
    limit: Option<usize>,
    offset: Option<usize>,
    count: Option<bool>,
    after: Option<String>,
    before: Option<String>,
}

impl PageRequest {
    fn parse(page: &Json) -> Result<Self, Failure> {
        let Json::Object(map) = page else {
            return Err(Failure::invalid_pagination(page));
        };
        let unknown: Vec<String> = map
            .keys()
            .map(|key| snake(key))
            .filter(|key| !["limit", "offset", "count", "after", "before"].contains(&key.as_str()))
            .map(|key| to_camel_case(&key))
            .collect();
        if !unknown.is_empty() {
            return Err(Failure::unknown_page_keys(unknown));
        }
        let number = |key: &str| map.get(key).and_then(Json::as_u64).map(|n| n as usize);
        let text = |key: &str| map.get(key).and_then(Json::as_str).map(str::to_string);
        Ok(Self { limit: number("limit"), offset: number("offset"), count: map.get("count").and_then(Json::as_bool), after: text("after"), before: text("before") })
    }
}

/// A get's equality filters, from the request's `getBy`: exactly its fields, scalar.
fn get_by_filters(names: &Names, resource: &ResourceDef, fields: &[&'static str], given: Option<&Json>) -> Result<Vec<Filter>, Failure> {
    let empty = Map::new();
    let given = given.and_then(Json::as_object).unwrap_or(&empty);
    let provided: HashMap<String, &Json> = given.iter().map(|(key, value)| (names.field_named(resource, key), value)).collect();
    let to_camel_case = |field: &str| names.field(resource, field);
    let allowed: Vec<String> = fields.iter().map(|field| to_camel_case(field)).collect();
    let extra: Vec<String> = provided.keys().filter(|key| !fields.contains(&key.as_str())).map(|key| to_camel_case(key)).collect();
    if !extra.is_empty() {
        return Err(Failure::unexpected_get_by_fields(extra, allowed));
    }
    let missing: Vec<String> = fields.iter().filter(|field| !provided.contains_key(**field)).map(|field| to_camel_case(field)).collect();
    if !missing.is_empty() {
        return Err(Failure::missing_get_by_fields(missing));
    }
    let non_scalar: Vec<String> = fields.iter().filter(|field| provided[**field].is_object() || provided[**field].is_array()).map(|field| to_camel_case(field)).collect();
    if !non_scalar.is_empty() {
        return Err(Failure::invalid_get_by(&non_scalar));
    }
    fields
        .iter()
        .map(|field| {
            let ty = field_type(resource, field).expect("get_by names attributes");
            let value = value_input(ty, provided[*field]).map_err(|e| Failure::from_error(&e).unwrap_or_else(|| Failure::new("invalid", "Invalid", e.to_string())))?;
            Ok(Filter::eq(*field, value))
        })
        .collect()
}

/// Reads `sort`'s fields on `query`, so a keyset cursor can hold a calculation or an
/// aggregate the page sorted by, not only an attribute.
fn keep_sort_fields(dest: &ResourceDef, query: &mut RelatedQuery, sort: &[Sort]) {
    for field in sort.iter().map(|sort| sort.field.as_str()) {
        if dest.attribute(field).is_some() {
            if let Some(select) = &mut query.select
                && !select.iter().any(|name| name == field)
            {
                select.push(field.to_string());
            }
        } else if dest.calculation(field).is_some() && !query.calculations.iter().any(|name| name == field) {
            query.calculations.push(field.to_string());
        } else if dest.aggregate(field).is_some() && !query.aggregates.iter().any(|name| name == field) {
            query.aggregates.push(field.to_string());
        }
    }
}

/// The sort a read's preparations give it.
fn prepared_sort(action: &ActionDef) -> Vec<Sort> {
    action
        .preparations
        .iter()
        .filter_map(|preparation| match *preparation {
            PreparationDef::Sort { field, descending } => Some(Sort { field: field.to_string(), descending, ..Sort::default() }),
            _ => None,
        })
        .collect()
}

/// The rows `query` reads of `resource`, selecting what `selection` needs, as the actor
/// may see them.
async fn read_rows<D: DataLayer>(ctx: &Context<D>, resource: &'static ResourceDef, selection: &Selection, query: CompiledQuery) -> ash_core::Result<Vec<FieldMap>> {
    let query = CompiledQuery {
        select: Some(selection.attributes(resource)),
        aggregates: selection.of(|name| resource.aggregate(name).is_some()),
        calculations: selection.of(|name| resource.calculation(name).is_some()),
        calculation_args: selection.calculation_args.clone(),
        ..query
    };
    let mut rows = ctx.data.run_query(resource, &query).await?;
    for row in &mut rows {
        ash_core::redact_fields(resource, ctx.actor.as_ref(), row)?;
    }
    Ok(rows)
}

/// A value as JSON, as AshTypescript formats its field's type: a float a number, a
/// decimal its text.
fn to_json(ty: Option<AttrType>, value: &Value) -> Json {
    match (ty, value) {
        (_, Value::Null) => Json::Null,
        // Each item as its type renders.
        (Some(AttrType::Array { of }), Value::Array(items)) => Json::Array(items.iter().map(|item| to_json(Some(*of), item)).collect()),
        // Declared fields, named as a client names them.
        (Some(ty @ (AttrType::Embedded(_) | AttrType::TypedMap { .. })), Value::Map(map)) => Json::Object(
            ty.fields()
                .unwrap_or_default()
                .iter()
                .filter_map(|field| map.get(field.name).map(|value| (to_camel_case(field.name), to_json(Some(field.ty), value))))
                .collect(),
        ),
        // The member it holds, as AshTypescript renders a union: `{member: value}`.
        (Some(AttrType::Union { members, .. }), value) => match value.union_member() {
            Some((name, held)) => {
                let ty = members.iter().find(|member| member.name == name).map(|member| member.ty);
                json!({ to_camel_case(name): to_json(ty, held) })
            }
            None => value.to_plain_json(),
        },
        (Some(AttrType::Float), Value::String(text)) => text.parse::<f64>().ok().and_then(serde_json::Number::from_f64).map_or_else(|| Json::String(text.clone()), Json::Number),
        (_, Value::Bool(b)) => Json::Bool(*b),
        (_, Value::Int(i)) => Json::from(*i),
        (_, Value::Float(n)) => serde_json::Number::from_f64(*n).map_or(Json::Null, Json::Number),
        (_, Value::String(s)) => Json::String(s.clone()),
        (_, Value::Uuid(u)) => Json::String(u.to_string()),
        (_, Value::Map(_) | Value::Array(_)) => value.to_plain_json(),
    }
}

/// `value` of type `ty` with only what `within` selects of it, as AshTypescript renders a
/// selection within an embedded resource, typed map or union: a union whose member isn't
/// selected is null.
fn select_value(ty: Option<AttrType>, value: &Value, within: &ValueSelection) -> Json {
    match (ty, value) {
        (_, Value::Null) => Json::Null,
        (Some(AttrType::Array { of }), Value::Array(items)) => {
            Json::Array(items.iter().map(|item| select_value(Some(*of), item, within)).collect())
        }
        (Some(ty), Value::Map(map)) if !matches!(ty, AttrType::Union { .. }) => {
            let ValueSelection::Fields(selected) = within else {
                return to_json(Some(ty), value);
            };
            let fields = ty.fields().unwrap_or_default();
            Json::Object(
                selected
                    .iter()
                    .map(|(name, nested)| {
                        let field_ty = fields.iter().find(|field| field.name == *name).map(|field| field.ty);
                        let value = map.get(*name).map_or(Json::Null, |value| match nested {
                            Some(nested) => select_value(field_ty, value, nested),
                            None => to_json(field_ty, value),
                        });
                        (to_camel_case(name), value)
                    })
                    .collect(),
            )
        }
        (Some(AttrType::Union { members, .. }), value) => {
            let (ValueSelection::Members(selected), Some((name, held))) = (within, value.union_member()) else {
                return Json::Null;
            };
            let Some((_, nested)) = selected.iter().find(|(member, _)| *member == name) else {
                return Json::Null;
            };
            let member_ty = members.iter().find(|member| member.name == name).map(|member| member.ty);
            let held = match nested {
                Some(nested) => select_value(member_ty, held, nested),
                None => to_json(member_ty, held),
            };
            json!({ to_camel_case(name): held })
        }
        (ty, value) => to_json(ty, value),
    }
}

#[cfg(feature = "axum")]
pub mod channel;

#[cfg(feature = "axum")]
pub mod axum {
    //! `POST /rpc/run` and `POST /rpc/validate`, each request in the context `context`
    //! makes of its headers: its actor and tenant.

    use std::sync::Arc;

    use ash_core::{Context, TransactionSupport};
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::routing::post;
    use axum::{Json, Router};

    use super::Rpc;

    /// The context a request runs in, made of its headers.
    type MakeContext<D> = Arc<dyn Fn(&HeaderMap) -> Context<D> + Send + Sync>;

    struct Served<D> {
        rpc: Rpc<D>,
        context: MakeContext<D>,
    }

    /// The RPC endpoints for `rpc`, each request in the context `context` makes of its
    /// headers.
    pub fn rpc_router<D: TransactionSupport + 'static>(
        rpc: Rpc<D>,
        context: impl Fn(&HeaderMap) -> Context<D> + Send + Sync + 'static,
    ) -> Router {
        let served = Arc::new(Served { rpc, context: Arc::new(context) });
        Router::new()
            .route("/rpc/run", post(run::<D>))
            .route("/rpc/validate", post(validate::<D>))
            .with_state(served)
    }

    async fn validate<D: TransactionSupport + 'static>(
        State(served): State<Arc<Served<D>>>,
        headers: HeaderMap,
        Json(request): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        let ctx = (served.context)(&headers);
        Json(served.rpc.validate(&ctx, &request).await)
    }

    async fn run<D: TransactionSupport + 'static>(
        State(served): State<Arc<Served<D>>>,
        headers: HeaderMap,
        Json(request): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        let ctx = (served.context)(&headers);
        Json(served.rpc.run(&ctx, &request).await)
    }
}
