//! Generic actions, as AshGraphql serves them (`action :name, :action` in its `queries`
//! or `mutations`): a query takes the action's arguments as its own, a mutation takes
//! them in an `input` object (`<Name>Input`), and either answers what the action
//! returns, or `true` for an action that returns nothing. A failure is raised at the top
//! level.

use std::sync::Arc;

use ash_core::{ActionDef, ActionKind, BoxFuture, Context, FieldMap, Resource, ResourceDef, TransactionSupport, Value};
use async_graphql::Value as GqlValue;
use async_graphql::dynamic::{Field, FieldFuture, FieldValue, InputObject, InputValue, ResolverContext, TypeRef};

use crate::error::raised;
use crate::names::{camel, pascal};
use crate::types::{ash_value_to_graphql_value_typed, attr_type_to_type_ref, parse_input_val};

/// Runs a generic action with its arguments, as its own `run` does.
pub(crate) type Runner<D> = Arc<dyn Fn(Context<D>, FieldMap) -> BoxFuture<'static, ash_core::Result<Value>> + Send + Sync>;

/// A generic action the schema serves.
pub(crate) struct GenericAction<D> {
    /// The field's name, as given (`route_ticket`); the schema camelizes it.
    pub name: String,
    pub resource: &'static ResourceDef,
    pub action: &'static ActionDef,
    pub mutation: bool,
    pub run: Runner<D>,
}

impl<D: TransactionSupport + Clone + 'static> GenericAction<D> {
    /// `R`'s generic action `action`, served as `name`, run as its own `run` runs it.
    pub(crate) fn of<R: Resource>(name: &str, action: &str, mutation: bool) -> Self {
        let resource = &R::DEF;
        let action = resource
            .action(action)
            .unwrap_or_else(|| panic!("`{name}` serves `{action}`, which isn't an action of {}", resource.name));
        assert!(action.kind == ActionKind::Generic, "`{name}` serves `{}`, which isn't a generic action", action.name);
        let action_name = action.name;
        let run: Runner<D> = Arc::new(move |ctx: Context<D>, input: FieldMap| -> BoxFuture<'static, ash_core::Result<Value>> {
            Box::pin(async move { R::run_generic(&ctx, action_name, input).await })
        });
        Self { name: name.to_string(), resource, action, mutation, run }
    }
}

impl<D> GenericAction<D> {
    fn input_name(&self) -> String {
        format!("{}Input", pascal(&self.name))
    }

    /// Whether some argument must be given.
    fn requires_input(&self) -> bool {
        self.action.arguments.iter().any(|arg| !arg.allow_nil && arg.default.is_none())
    }

    fn argument_type(&self, arg: &ash_core::ArgumentDef) -> TypeRef {
        attr_type_to_type_ref(self.resource.name, arg.name, arg.ty, arg.allow_nil || arg.default.is_some())
    }

    /// A mutation's `<Name>Input`, if the action takes arguments.
    pub(crate) fn input_object(&self) -> Option<InputObject> {
        if !self.mutation || self.action.arguments.is_empty() {
            return None;
        }
        let mut input = InputObject::new(self.input_name());
        for arg in self.action.arguments {
            input = input.field(InputValue::new(camel(arg.name), self.argument_type(arg)));
        }
        Some(input)
    }
}

impl<D: Send + Sync + 'static> GenericAction<D> {
    /// The query or mutation field.
    pub(crate) fn field(self) -> Field {
        // What the action returns, or `true` for nothing, as AshGraphql's `Boolean`.
        let type_ref = match self.action.returns {
            Some(ty) => attr_type_to_type_ref(self.resource.name, self.action.name, ty, false),
            None => TypeRef::named_nn(TypeRef::BOOLEAN),
        };
        let (action, mutation, run) = (self.action, self.mutation, Arc::clone(&self.run));
        let mut field = Field::new(camel(&self.name), type_ref, move |ctx| {
            let run = Arc::clone(&run);
            FieldFuture::new(async move {
                let ash = crate::request::request_context::<D>(&ctx)?.into_owned();
                let input = arguments(&ctx, action, mutation)?;
                // Every failure, as AshGraphql raises each of an action's errors.
                let value = match run(ash, input).await {
                    Ok(value) => value,
                    Err(err) => {
                        let mut errors = err.each().into_iter();
                        let first = errors.next().map(raised).unwrap_or_else(|| raised(&err));
                        for error in errors {
                            let error = raised(error).into_server_error(ctx.ctx.item.pos);
                            ctx.ctx.add_error(ctx.ctx.set_error_path(error));
                        }
                        return Err(first);
                    }
                };
                Ok(Some(FieldValue::value(match action.returns {
                    Some(ty) => ash_value_to_graphql_value_typed(&value, ty),
                    None => GqlValue::Boolean(true),
                })))
            })
        });
        if mutation {
            if !self.action.arguments.is_empty() {
                let input = self.input_name();
                let required = self.requires_input();
                field = field.argument(InputValue::new(
                    "input",
                    if required { TypeRef::named_nn(input) } else { TypeRef::named(input) },
                ));
            }
        } else {
            for arg in self.action.arguments {
                field = field.argument(InputValue::new(camel(arg.name), self.argument_type(arg)));
            }
        }
        field
    }
}

/// The action's arguments as given: a query's own, or a mutation's `input`.
fn arguments(ctx: &ResolverContext<'_>, action: &ActionDef, mutation: bool) -> async_graphql::Result<FieldMap> {
    let mut input = FieldMap::new();
    let given = if mutation {
        match ctx.args.get("input").filter(|value| !value.is_null()) {
            Some(given) => Some(given.object()?),
            None => None,
        }
    } else {
        None
    };
    for arg in action.arguments {
        let value = match &given {
            Some(given) => given.get(&camel(arg.name)),
            None if !mutation => ctx.args.get(&camel(arg.name)),
            None => None,
        };
        if let Some(value) = value.filter(|value| !value.is_null()) {
            input.insert(arg.name.to_string(), parse_input_val(&value, arg.ty)?);
        }
    }
    Ok(input)
}
