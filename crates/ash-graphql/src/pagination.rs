//! Pages, as AshGraphql returns a paginated read: a keyset page, `KeysetPageOf<Resource> {
//! count, results, startKeyset, endKeyset }`, paged with `first` / `after` and `last` /
//! `before`, or an offset page, `PageOf<Resource> { count, results, hasNextPage, ... }`,
//! paged with `limit` and `offset`. Which a read gives follows its action's `pagination`.

use ash_core::{
    ActionDef, ActionKind, CompiledQuery, Countable, DataLayer, FieldMap, Filter, KeysetCursor, Pagination,
    ResourceDef, Sort, Value, build_keyset_filter, keyset_sort, keyset_values, scope_read,
};
use async_graphql::Value as GqlValue;
use async_graphql::dynamic::*;

use crate::redact::redact_record;
use crate::filter::{parse_resource_filter, resource_filter_input_name};
use crate::names::camel;
use crate::preload::{Load, preload, selected};
use crate::request::request_context;
use crate::sort::{parse_resource_sort, resource_sort_input_name};
use crate::types::parse_input_val;

/// The most records a page holds, as Ash's default `max_page_size`.
pub const MAX_PAGE_SIZE: usize = 250;

/// How a read's query pages, as AshGraphql chooses for a list query (`paginate_with`
/// defaulting to keyset): by keyset where its action pages by keyset, else by offset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageStrategy {
    Keyset,
    Offset,
}

impl PageStrategy {
    /// How `action` pages, or `None` when it doesn't: its query is a plain list.
    pub fn of(action: &ActionDef) -> Option<Self> {
        let pagination = action.pagination.as_ref()?;
        Some(if pagination.keyset || !pagination.offset { Self::Keyset } else { Self::Offset })
    }
}

/// How the reads of `resource` page: its read actions', or its implicit read's.
fn read_paginations(resource: &ResourceDef) -> impl Iterator<Item = Option<&Pagination>> {
    let reads = resource.actions.iter().filter(|a| a.kind == ActionKind::Read);
    reads.chain(std::iter::once(resource.default_read())).map(|a| a.pagination.as_ref())
}

/// Whether some read of `resource` counts, so its pages have a `count`, as AshGraphql's.
fn countable(resource: &ResourceDef) -> bool {
    read_paginations(resource).any(|p| p.is_some_and(|p| p.countable != Countable::No))
}

/// A page of records and the keysets at its ends.
#[derive(Clone)]
pub struct KeysetPage {
    pub results: Vec<FieldMap>,
    pub start_keyset: Option<String>,
    pub end_keyset: Option<String>,
    pub count: Option<usize>,
}

/// `Trip` → `KeysetPageOfTrip`.
pub fn keyset_page_type_name(resource_name: &str) -> String {
    format!("KeysetPageOf{resource_name}")
}

/// An offset page of records, and where it is among the pages.
#[derive(Clone)]
pub struct OffsetPage {
    pub results: Vec<FieldMap>,
    pub count: Option<usize>,
    pub limit: usize,
    pub offset: usize,
    /// Whether records follow this page.
    pub more: bool,
}

impl OffsetPage {
    /// Pages in all, as AshGraphql counts them: one when uncounted.
    fn last_page(&self) -> usize {
        match self.count {
            None | Some(0) => 1,
            Some(count) => count.div_ceil(self.limit),
        }
    }

    fn page_number(&self) -> usize {
        (self.offset.div_ceil(self.limit) + 1).min(self.last_page())
    }
}

/// `Trip` → `PageOfTrip`.
pub fn offset_page_type_name(resource_name: &str) -> String {
    format!("PageOf{resource_name}")
}

fn int(n: usize) -> FieldValue<'static> {
    FieldValue::value(GqlValue::Number((n as i64).into()))
}

/// Registers the page types a query of `resource` returns, `KeysetPageOf<Resource>` and
/// `PageOf<Resource>` (as `strategies` holds them), as AshGraphql's schema has them: with
/// a `count` where some read of the resource counts. Their fields borrow the page, so a
/// page of records is never copied to resolve it.
pub fn register_pages(
    mut builder: SchemaBuilder,
    resource: &'static ResourceDef,
    strategies: &[PageStrategy],
) -> SchemaBuilder {
    fn page<'a>(ctx: &ResolverContext<'a>) -> Option<&'a KeysetPage> {
        ctx.parent_value.downcast_ref::<KeysetPage>()
    }
    fn offset_page<'a>(ctx: &ResolverContext<'a>) -> Option<&'a OffsetPage> {
        ctx.parent_value.downcast_ref::<OffsetPage>()
    }
    fn keyset(value: &Option<String>) -> Option<FieldValue<'static>> {
        value.as_ref().map(|v| FieldValue::value(GqlValue::String(v.clone())))
    }
    let countable = countable(resource);

    let mut keyset_page = Object::new(keyset_page_type_name(resource.name))
        .description(format!("A keyset page of {}", resource.name))
        .field(Field::new("results", TypeRef::named_nn_list(resource.name), |ctx| {
            FieldFuture::new(async move {
                Ok(page(&ctx).map(|p| FieldValue::list(p.results.iter().map(|r| FieldValue::borrowed_any(r)))))
            })
        }))
        .field(Field::new("startKeyset", TypeRef::named(TypeRef::STRING), |ctx| {
            FieldFuture::new(async move { Ok(page(&ctx).and_then(|p| keyset(&p.start_keyset))) })
        }))
        .field(Field::new("endKeyset", TypeRef::named(TypeRef::STRING), |ctx| {
            FieldFuture::new(async move { Ok(page(&ctx).and_then(|p| keyset(&p.end_keyset))) })
        }));

    let mut page_of = Object::new(offset_page_type_name(resource.name))
        .description(format!("A page of {}", resource.name))
        .field(Field::new("results", TypeRef::named_nn_list(resource.name), |ctx| {
            FieldFuture::new(async move {
                Ok(offset_page(&ctx).map(|p| FieldValue::list(p.results.iter().map(|r| FieldValue::borrowed_any(r)))))
            })
        }));

    if countable {
        keyset_page = keyset_page.field(Field::new("count", TypeRef::named(TypeRef::INT), |ctx| {
            FieldFuture::new(async move { Ok(page(&ctx).and_then(|p| p.count).map(int)) })
        }));
        page_of = page_of
            .field(Field::new("count", TypeRef::named(TypeRef::INT), |ctx| {
                FieldFuture::new(async move { Ok(offset_page(&ctx).and_then(|p| p.count).map(int)) })
            }))
            // Where the page is, as AshGraphql adds it to a countable page.
            .field(Field::new("hasNextPage", TypeRef::named_nn(TypeRef::BOOLEAN), |ctx| {
                FieldFuture::new(async move { Ok(offset_page(&ctx).map(|p| FieldValue::value(GqlValue::Boolean(p.more)))) })
            }))
            .field(Field::new("hasPreviousPage", TypeRef::named_nn(TypeRef::BOOLEAN), |ctx| {
                FieldFuture::new(async move {
                    Ok(offset_page(&ctx).map(|p| FieldValue::value(GqlValue::Boolean(p.offset > 0))))
                })
            }))
            .field(Field::new("pageNumber", TypeRef::named_nn(TypeRef::INT), |ctx| {
                FieldFuture::new(async move { Ok(offset_page(&ctx).map(|p| int(p.page_number()))) })
            }))
            .field(Field::new("lastPage", TypeRef::named_nn(TypeRef::INT), |ctx| {
                FieldFuture::new(async move { Ok(offset_page(&ctx).map(|p| int(p.last_page()))) })
            }))
            .field(Field::new("limit", TypeRef::named_nn(TypeRef::INT), |ctx| {
                FieldFuture::new(async move { Ok(offset_page(&ctx).map(|p| int(p.limit))) })
            }));
    }
    if strategies.contains(&PageStrategy::Keyset) {
        builder = builder.register(keyset_page);
    }
    if strategies.contains(&PageStrategy::Offset) {
        builder = builder.register(page_of);
    }
    builder
}

/// The action arguments a read query takes, parsed.
pub(crate) fn read_arguments(
    ctx: &ResolverContext<'_>,
    action: &ActionDef,
) -> async_graphql::Result<FieldMap> {
    let mut arguments = FieldMap::new();
    for arg in action.arguments {
        if let Some(value) = ctx.args.get(&camel(arg.name))
            && !value.is_null()
        {
            let value = parse_input_val(&value, arg.ty)?;
            if !value.is_null() {
                arguments.insert(arg.name.to_string(), value);
            }
        }
    }
    Ok(arguments)
}

/// Calculations take the action arguments named like theirs.
pub(crate) fn calculation_arguments(
    resource: &ResourceDef,
    arguments: &FieldMap,
) -> std::collections::HashMap<String, FieldMap> {
    let mut out = std::collections::HashMap::new();
    for calc in resource.calculations {
        let matched: FieldMap = calc
            .arguments
            .iter()
            .filter_map(|arg| arguments.get(arg.name).map(|val| (arg.name.to_string(), val.clone())))
            .collect();
        if !matched.is_empty() {
            out.insert(calc.name.to_string(), matched);
        }
    }
    out
}

/// Adds a read action's arguments, and `sort` and `filter`, to a query field.
pub(crate) fn read_field_arguments(
    mut field: Field,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
) -> Field {
    field = field
        .argument(InputValue::new(
            "sort",
            TypeRef::named_list(resource_sort_input_name(resource.name)),
        ))
        .argument(InputValue::new(
            "filter",
            TypeRef::named(resource_filter_input_name(resource.name)),
        ));
    for arg in action.arguments {
        let type_ref = crate::types::input_type_ref(resource.name, arg.name, arg.ty, arg.allow_nil || arg.default.is_some());
        field = field.argument(InputValue::new(camel(arg.name), type_ref));
    }
    field
}

/// A read query's `filter`, `sort` and action arguments, as the query its action runs:
/// with the action's preparations and argument filters, the actor's read policies and
/// the context's tenant.
pub(crate) fn scoped_read<D: DataLayer>(
    ctx: &ResolverContext<'_>,
    ash: &ash_core::Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
) -> async_graphql::Result<(FieldMap, CompiledQuery)> {
    let arguments = read_arguments(ctx, action)?;
    let filter = match ctx.args.get("filter").filter(|value| !value.is_null()) {
        Some(filter) => Some(parse_resource_filter(resource, ash.actor.as_ref(), filter.as_value())?),
        None => None,
    };
    let sort = match ctx.args.get("sort").filter(|value| !value.is_null()) {
        Some(sort) => parse_resource_sort(resource, ash.actor.as_ref(), sort.as_value())?,
        None => Vec::new(),
    };
    let query = scope_read(
        resource,
        action,
        ash.actor.as_ref(),
        &arguments,
        CompiledQuery {
            filter,
            sort,
            calculation_args: calculation_arguments(resource, &arguments),
            tenant: ash.tenant.clone(),
            ..CompiledQuery::default()
        },
    )
    .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    Ok((arguments, query))
}

/// The records a read through `action` found, after its `after_action` preparations.
pub(crate) fn after_read(action: &ActionDef, arguments: &FieldMap, records: &mut [FieldMap]) -> async_graphql::Result<()> {
    ash_core::after_read(action, arguments, records).map_err(|e| async_graphql::Error::new(e.to_string()))
}

/// The number a page argument gives, if any.
fn number_arg(ctx: &ResolverContext<'_>, name: &str) -> async_graphql::Result<Option<i64>> {
    ctx.args.get(name).filter(|v| !v.is_null()).map(|v| v.i64()).transpose()
}

/// How `action` pages: its own `pagination`, or Ash's keyset defaults for an action
/// that doesn't declare one.
fn pagination_of(action: &ActionDef) -> Pagination {
    action.pagination.unwrap_or(Pagination::keyset())
}

/// Counts the records `scoped` finds, for a page's `count`.
async fn count_records<D: DataLayer>(
    ash: &ash_core::Context<D>,
    resource: &ResourceDef,
    scoped: &CompiledQuery,
) -> async_graphql::Result<usize> {
    let query = CompiledQuery { sort: Vec::new(), limit: None, offset: None, ..scoped.clone() };
    ash.data.count(resource, &query).await.map_err(|e| async_graphql::Error::new(e.to_string()))
}

/// A keyset-paginated read through `action`: `<name>(sort, filter, first, before, after,
/// last, <arguments>): KeysetPageOf<Resource>`. Given no `first` or `last`, a page holds
/// the action's default limit, or as many records as it may.
pub fn build_keyset_query<D: DataLayer + Clone + 'static>(
    name: String,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
) -> Field {
    let pk_name = resource
        .attributes
        .iter()
        .find(|a| a.primary_key)
        .map(|a| a.name)
        .unwrap_or("id");
    let pagination = pagination_of(action);

    let field = Field::new(name, TypeRef::named(keyset_page_type_name(resource.name)), move |ctx| {
        FieldFuture::new(async move {
            let ash = request_context::<D>(&ctx)?;
            let keyset = |name: &str| ctx.args.get(name).and_then(|v| v.string().ok().map(str::to_string));
            let (first, last) = (number_arg(&ctx, "first")?, number_arg(&ctx, "last")?);
            let (after, before) = (keyset("after"), keyset("before"));
            let limit = page_limit(first, last, after.is_some(), before.is_some(), &pagination)?;
            let backward = last.is_some() || (before.is_some() && after.is_none());

            let (arguments, scoped) = scoped_read(&ctx, &ash, resource, action)?;

            // A page sorts stably, so every record has its own keyset.
            let sort = keyset_sort(resource, scoped.sort.clone());

            // Counted alongside the page, as Ash reads a page's count.
            let count_scope = ctx.look_ahead().field("count").exists().then(|| scoped.clone());

            let mut page_filter = scoped.filter.clone();
            let cursor = if backward { before.as_deref() } else { after.as_deref() };
            if let Some(cursor) = cursor.and_then(KeysetCursor::decode) {
                let values = keyset_values(resource, &cursor, &sort);
                if let Some(keyset_filter) = build_keyset_filter(resource, &sort, &values, !backward) {
                    page_filter = Some(match page_filter {
                        Some(f) => Filter::And(vec![f, keyset_filter]),
                        None => keyset_filter,
                    });
                }
            }

            // A backward page reads in reverse, then flips.
            let mut query_sort = sort.clone();
            if backward {
                for s in &mut query_sort {
                    *s = s.reversed();
                }
            }
            // Only what's selected, and the sort a keyset holds.
            let load = Load::of(
                resource,
                &selected(ctx.ctx.field(), Some("results")),
                sort.iter().map(|s| s.field.as_str()),
            );
            let query = load.onto(CompiledQuery {
                filter: page_filter,
                sort: query_sort,
                limit,
                offset: None,
                ..scoped
            });
            // The count is read alongside the page, as Ash reads a page's count. In a
            // transaction, its one connection takes them in turn.
            let counting = async {
                match &count_scope {
                    Some(scoped) => count_records(&ash, resource, scoped).await.map(Some),
                    None => Ok(None),
                }
            };
            let reading = async {
                let mut records = ash
                    .data
                    .run_query(resource, &query)
                    .await
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;
                if backward {
                    records.reverse();
                }
                after_read(action, &arguments, &mut records)?;
                for record in &mut records {
                    redact_record(resource, ash.actor.as_ref(), record);
                }
                preload(&ash, resource, selected(ctx.ctx.field(), Some("results")), &mut records).await?;
                Ok::<_, async_graphql::Error>(records)
            };
            let (count, records) = futures_util::future::try_join(counting, reading).await?;
            let keyset = |record: Option<&FieldMap>| record.map(|r| keyset_of(r, &sort, pk_name));
            Ok(Some(FieldValue::owned_any(KeysetPage {
                start_keyset: keyset(records.first()),
                end_keyset: keyset(records.last()),
                results: records,
                count,
            })))
        })
    });

    read_field_arguments(field, resource, action)
        .argument(InputValue::new("first", TypeRef::named(TypeRef::INT)))
        .argument(InputValue::new("before", TypeRef::named(TypeRef::STRING)))
        .argument(InputValue::new("after", TypeRef::named(TypeRef::STRING)))
        .argument(InputValue::new("last", TypeRef::named(TypeRef::INT)))
}

/// How many records a page holds, as AshGraphql validates a keyset read's arguments:
/// `first` records after `after`, `last` before `before`, or, given neither, the action's
/// default limit, else as many as a page may hold. Never more than that; `None` where
/// nothing limits it.
fn page_limit(
    first: Option<i64>,
    last: Option<i64>,
    after: bool,
    before: bool,
    pagination: &Pagination,
) -> async_graphql::Result<Option<usize>> {
    let error = |message: &str| Err(async_graphql::Error::new(message));
    let cursors = "You can pass either `first` and `after` cursor, or `last` and `before` cursor";
    let capped = |n: i64| Some(capped_limit(n as usize, pagination));
    match (first, last) {
        (Some(_), Some(_)) => error("You can pass either `first` or `last`, not both"),
        (Some(_), _) if before => error(cursors),
        (_, Some(_)) if after => error(cursors),
        (Some(n), _) if n < 1 => error("`first` must be a positive integer"),
        (_, Some(n)) if n < 1 => error("`last` must be a positive integer"),
        (Some(n), _) => Ok(capped(n)),
        (_, Some(n)) if before => Ok(capped(n)),
        (_, Some(_)) => error("You can pass `last` only with `before` cursor"),
        (None, None) => Ok(pagination.default_limit.or(pagination.max_page_size)),
    }
}

fn capped_limit(limit: usize, pagination: &Pagination) -> usize {
    pagination.max_page_size.map_or(limit, |max| limit.min(max))
}

/// An offset-paginated read through `action`: `<name>(sort, filter, limit, offset,
/// <arguments>): PageOf<Resource>`, as AshGraphql gives a read that pages by offset only.
/// Given no `limit`, a page holds the action's default limit, or as many records as it
/// may.
pub fn build_offset_query<D: DataLayer + Clone + 'static>(
    name: String,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
) -> Field {
    let pagination = pagination_of(action);
    let field = Field::new(name, TypeRef::named(offset_page_type_name(resource.name)), move |ctx| {
        FieldFuture::new(async move {
            let ash = request_context::<D>(&ctx)?;
            let limit = match number_arg(&ctx, "limit")? {
                Some(n) if n < 1 => return Err(async_graphql::Error::new("`limit` must be a positive integer")),
                Some(n) => Some(capped_limit(n as usize, &pagination)),
                None => pagination.default_limit.or(pagination.max_page_size),
            };
            let offset = number_arg(&ctx, "offset")?.unwrap_or(0).max(0) as usize;
            let (arguments, scoped) = scoped_read(&ctx, &ash, resource, action)?;
            let count = if ctx.look_ahead().field("count").exists() {
                Some(count_records(&ash, resource, &scoped).await?)
            } else {
                None
            };
            let fields = selected(ctx.ctx.field(), Some("results"));
            let load = Load::of(resource, &fields, scoped.sort.iter().map(|s| s.field.as_str()));
            // One more than the page, to know whether more follow.
            let query = load.onto(CompiledQuery { limit: limit.map(|l| l + 1), offset: Some(offset), ..scoped });
            let mut records = ash
                .data
                .run_query(resource, &query)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            let more = limit.is_some_and(|l| records.len() > l);
            if let Some(limit) = limit {
                records.truncate(limit);
            }
            after_read(action, &arguments, &mut records)?;
            for record in &mut records {
                redact_record(resource, ash.actor.as_ref(), record);
            }
            preload(&ash, resource, fields, &mut records).await?;
            Ok(Some(FieldValue::owned_any(OffsetPage {
                limit: limit.unwrap_or(records.len()).max(1),
                results: records,
                count,
                offset,
                more,
            })))
        })
    });
    // `limit` is required where every read pages and nothing defaults it.
    let mut limit = InputValue::new(
        "limit",
        if pagination.required && pagination.default_limit.is_none() {
            TypeRef::named_nn(TypeRef::INT)
        } else {
            TypeRef::named(TypeRef::INT)
        },
    );
    if let Some(default) = pagination.default_limit {
        limit = limit.default_value(GqlValue::Number((default as i64).into()));
    }
    read_field_arguments(field, resource, action)
        .argument(limit)
        .argument(InputValue::new("offset", TypeRef::named(TypeRef::INT)))
}

fn keyset_of(record: &FieldMap, sort: &[Sort], pk_name: &str) -> String {
    let id = record.get(pk_name).cloned().unwrap_or(Value::Null);
    let values = sort
        .iter()
        .map(|s| (s.field.clone(), record.get(&s.field).cloned().unwrap_or(Value::Null)))
        .collect();
    KeysetCursor { id, values }.encode()
}
