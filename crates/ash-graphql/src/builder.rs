use ash_core::{ActionKind, Context, DataLayer, DomainDef, ResourceDef};
use ash_pubsub::PubSub;
use async_graphql::dynamic::*;

use crate::error::register_user_error;
use crate::filter::register_resource_filter_inputs;
use crate::mutation::{
    build_action_mutation, register_action_input, register_action_payload,
};
use crate::object::{build_resource_object, collect_enums_for_resource};
use crate::pagination::{PageStrategy, register_pages};
use crate::query::{build_read_action_query, build_resource_queries};
use crate::sort::{register_resource_sort_inputs, register_sort_order};
use crate::subscription::{build_resource_subscriptions, register_subscription_results};

/// High-level builder for creating an `async-graphql` [`Schema`] from Ash domains and resources.
pub struct AshGraphQLBuilder {
    pub(crate) resources: Vec<&'static ResourceDef>,
    pub(crate) pubsub: Option<PubSub>,
    pub(crate) dataloader_enabled: bool,
}

impl AshGraphQLBuilder {
    /// Creates a new builder from an Ash [`DomainDef`].
    pub fn from_domain(domain: &'static DomainDef) -> Self {
        Self {
            resources: domain.resources.to_vec(),
            pubsub: None,
            dataloader_enabled: false,
        }
    }

    /// Creates a new builder from a list of [`ResourceDef`]s.
    pub fn from_resources(resources: &[&'static ResourceDef]) -> Self {
        Self {
            resources: resources.to_vec(),
            pubsub: None,
            dataloader_enabled: false,
        }
    }

    /// Adds live subscriptions (`<resource>Created`, `Updated`, `Destroyed`) fed by
    /// `pubsub`.
    ///
    /// Changes reach it only through notifiers, as every other notification does: give
    /// the contexts your writes run in a `PubSubNotifier` on the same `PubSub` (for
    /// example with `ash_pubsub::ContextPubSubExt::with_pubsub`). Then every write,
    /// through GraphQL or not, is published once, after its transaction commits.
    pub fn with_pubsub(mut self, pubsub: PubSub) -> Self {
        self.pubsub = Some(pubsub);
        self
    }

    /// Batches relationship resolution with a `DataLoader` per request, bound to the
    /// `Context<D>` (and `Actor`) that request runs as.
    pub fn with_dataloader(mut self) -> Self {
        self.dataloader_enabled = true;
        self
    }

    /// Builds the dynamic GraphQL schema for the specified data layer context type `D`.
    pub fn finish<D: DataLayer + Clone + 'static>(self) -> Result<Schema, SchemaError> {
        self.finish_internal::<D>(None)
    }

    /// Builds the dynamic GraphQL schema and attaches the default [`Context`] to the schema data.
    pub fn finish_with_context<D: DataLayer + Clone + 'static>(
        self,
        ctx: Context<D>,
    ) -> Result<Schema, SchemaError> {
        self.finish_internal::<D>(Some(ctx))
    }

    fn finish_internal<D: DataLayer + Clone + 'static>(
        self,
        default_ctx: Option<Context<D>>,
    ) -> Result<Schema, SchemaError> {
        // Absinthe's root type names, as AshGraphql's schemas have them.
        let mut query = Object::new(ROOT_QUERY);
        // The pages each resource's queries return.
        let mut pages: Vec<(&str, PageStrategy)> = Vec::new();
        for res in &self.resources {
            let (get_field, list_field) = build_resource_queries::<D>(res);
            query = query.field(get_field).field(list_field);
            pages.extend(PageStrategy::of(res.default_read()).map(|s| (res.name, s)));
            for action in res.actions {
                if action.kind == ActionKind::Read && !action.primary && action.name != "read" {
                    query = query.field(build_read_action_query::<D>(action, res));
                    pages.extend(PageStrategy::of(action).map(|s| (res.name, s)));
                }
            }
        }

        let mut mutation = Object::new(ROOT_MUTATION);
        let mut has_mutations = false;
        for res in &self.resources {
            for action in res.actions {
                if matches!(action.kind, ActionKind::Create | ActionKind::Update | ActionKind::Destroy) {
                    has_mutations = true;
                    mutation = mutation.field(build_action_mutation::<D>(action, res));
                }
            }
        }

        let mut subscription = Subscription::new(ROOT_SUBSCRIPTION);
        let has_subscriptions = self.pubsub.is_some();
        if let Some(pubsub) = &self.pubsub {
            for res in &self.resources {
                for field in build_resource_subscriptions::<D>(res, pubsub.clone()) {
                    subscription = subscription.field(field);
                }
            }
        }

        let mut builder = Schema::build(
            ROOT_QUERY,
            has_mutations.then_some(ROOT_MUTATION),
            has_subscriptions.then_some(ROOT_SUBSCRIPTION),
        );
        builder = register_user_error(builder);
        builder = register_sort_order(builder);

        // Every reachable resource: those given, and the destinations of their relationships.
        let mut all_resources = self.resources.clone();
        let mut i = 0;
        while i < all_resources.len() {
            for rel in all_resources[i].relationships {
                let dest = (rel.destination)();
                if !all_resources.iter().any(|r| r.name == dest.name) {
                    all_resources.push(dest);
                }
            }
            i += 1;
        }

        // The custom scalars the schema uses; `Json` always, for mutation errors' `vars`.
        let used: Vec<&str> = all_resources
            .iter()
            .flat_map(|res| {
                res.attributes
                    .iter()
                    .map(|attr| attr.ty)
                    .chain(res.aggregates.iter().map(|agg| agg.ty))
                    .chain(res.calculations.iter().map(|calc| calc.ty))
                    .chain(res.actions.iter().flat_map(|a| a.arguments.iter().map(|arg| arg.ty)))
            })
            .map(crate::types::graphql_type_name)
            .collect();
        for scalar in crate::types::CUSTOM_SCALARS {
            if *scalar == "Json" || used.contains(scalar) {
                builder = builder.register(Scalar::new(*scalar));
            }
        }

        let mut enums = Vec::new();
        for res in &all_resources {
            builder = builder.register(build_resource_object::<D>(res));
            let strategies: Vec<PageStrategy> =
                pages.iter().filter(|(name, _)| *name == res.name).map(|(_, s)| *s).collect();
            builder = register_pages(builder, res, &strategies);
            builder = register_resource_filter_inputs(builder, res);
            builder = register_resource_sort_inputs(builder, res);
            if has_subscriptions {
                builder = register_subscription_results::<D>(builder, res);
            }
            for action in res.actions {
                if matches!(action.kind, ActionKind::Create | ActionKind::Update | ActionKind::Destroy) {
                    builder = register_action_input(builder, action, res);
                    builder = register_action_payload(builder, action, res);
                }
            }
            for e in collect_enums_for_resource(res) {
                let name = e.type_name().to_string();
                if !enums.contains(&name) {
                    enums.push(name);
                    builder = builder.register(e);
                }
            }
        }

        if has_mutations {
            builder = builder.register(mutation);
        }
        if has_subscriptions {
            builder = builder.register(subscription);
        }
        if self.dataloader_enabled {
            builder = builder.extension(crate::request::RequestDataLoader::<D>::new());
        }
        if let Some(ctx) = default_ctx {
            builder = builder.data(ctx);
        }
        builder.register(query).finish()
    }
}

/// The root types' names, as Absinthe gives them.
pub const ROOT_QUERY: &str = "RootQueryType";
pub const ROOT_MUTATION: &str = "RootMutationType";
pub const ROOT_SUBSCRIPTION: &str = "RootSubscriptionType";
