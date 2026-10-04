use crate::actor::Actor;
use crate::error::{Error, Result};
use crate::resource::AttrType;
use crate::value::{ConstValue, FieldMap};

/// Dynamic hook running before persistence with mutable access to attributes.
pub type DynamicBeforeActionHook = Box<dyn FnOnce(&mut FieldMap) -> Result<()> + Send + 'static>;

/// Dynamic hook running immediately after persistence with mutable access to persisted attributes.
pub type DynamicAfterActionHook = Box<dyn FnOnce(&mut FieldMap) -> Result<()> + Send + 'static>;

/// Dynamic hook running after transaction completion (commit or rollback), receiving the result.
pub type DynamicAfterTransactionHook =
    Box<dyn FnOnce(std::result::Result<&FieldMap, &Error>) + Send + 'static>;

/// Static function pointer for a `before_action` hook on an action definition.
pub type BeforeActionFn = fn(&mut FieldMap) -> Result<()>;

/// Static function pointer for an `after_action` hook on an action definition.
pub type AfterActionFn = fn(&mut FieldMap) -> Result<()>;

/// Static function pointer for an `after_transaction` hook on an action definition.
pub type AfterTransactionFn = fn(std::result::Result<&FieldMap, &Error>);

/// Target for an update or destroy action, which can be an entity ID, an existing record reference, or an owned record.
///
/// ### Snapshot vs. Refetch Semantics
/// - Passing a [`uuid::Uuid`] (e.g. `Ticket::assign(&ctx, ticket.id)`) refetches the latest record from the data layer.
/// - Passing a record reference or owned record (e.g. `Ticket::assign(&ctx, &ticket)`) operates directly on the provided snapshot,
///   avoiding an extra query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionTarget<R> {
    Id(uuid::Uuid),
    Record(R),
}

impl<R: crate::resource::Resource> ActionTarget<R> {
    /// Returns the entity ID of this target, whether given as an ID or extracted from an existing record.
    pub fn id(&self) -> uuid::Uuid {
        match self {
            Self::Id(id) => *id,
            Self::Record(rec) => rec.id(),
        }
    }
}

impl<R> From<uuid::Uuid> for ActionTarget<R> {
    fn from(id: uuid::Uuid) -> Self {
        Self::Id(id)
    }
}

impl<R> From<&uuid::Uuid> for ActionTarget<R> {
    fn from(id: &uuid::Uuid) -> Self {
        Self::Id(*id)
    }
}

impl<R: crate::resource::Resource + Clone> From<&R> for ActionTarget<R> {
    fn from(rec: &R) -> Self {
        Self::Record(rec.clone())
    }
}

impl<R: crate::resource::Resource + Clone> From<&mut R> for ActionTarget<R> {
    fn from(rec: &mut R) -> Self {
        Self::Record(rec.clone())
    }
}

impl<R: crate::resource::Resource> From<R> for ActionTarget<R> {
    fn from(rec: R) -> Self {
        Self::Record(rec)
    }
}

pub struct ChangeContext<'a> {
    pub fields: &'a mut FieldMap,
    pub actor: Option<&'a Actor>,
    pub tenant: Option<&'a str>,
    pub metadata: &'a FieldMap,
    pub arguments: &'a FieldMap,
    pub before_actions: &'a mut Vec<DynamicBeforeActionHook>,
    pub after_actions: &'a mut Vec<DynamicAfterActionHook>,
    pub after_transactions: &'a mut Vec<DynamicAfterTransactionHook>,
}

impl<'a> ChangeContext<'a> {
    /// Register a hook to run immediately before persistence.
    /// May inspect or mutate attributes, or return an error to abort the write.
    pub fn before_action<F>(&mut self, hook: F)
    where
        F: FnOnce(&mut FieldMap) -> Result<()> + Send + 'static,
    {
        self.before_actions.push(Box::new(hook));
    }

    /// Register a hook to run immediately after persistence within the transaction.
    /// Receives mutable access to the newly saved record attributes.
    pub fn after_action<F>(&mut self, hook: F)
    where
        F: FnOnce(&mut FieldMap) -> Result<()> + Send + 'static,
    {
        self.after_actions.push(Box::new(hook));
    }

    /// Register a hook to run after the transaction finishes (or immediately if not transactional).
    /// Receives the final result (`Ok(&fields)` or `Err(&error)`).
    pub fn after_transaction<F>(&mut self, hook: F)
    where
        F: FnOnce(std::result::Result<&FieldMap, &Error>) + Send + 'static,
    {
        self.after_transactions.push(Box::new(hook));
    }
}

pub trait CustomChange: Send + Sync + 'static {
    fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()>;

    /// What the change sets in an atomic update, as Ash's `atomic/3`: expressions over the
    /// record as the data layer holds it, and conditions on it. By default a custom change
    /// can't run atomically.
    fn atomic(&self, _ctx: &crate::atomic::AtomicContext<'_>) -> crate::atomic::Atomic {
        crate::atomic::Atomic::not_atomic("a custom change without an atomic implementation")
    }
}

pub struct ValidationContext<'a> {
    pub record: Option<&'a FieldMap>,
    pub fields: &'a FieldMap,
    pub actor: Option<&'a Actor>,
    pub tenant: Option<&'a str>,
    pub metadata: &'a FieldMap,
    pub arguments: &'a FieldMap,
}

pub trait CustomValidation: Send + Sync + 'static {
    fn validate(&self, ctx: &ValidationContext<'_>) -> Result<()>;

    /// When the validation fails in an atomic update, as Ash's `atomic/3`: conditions on
    /// the record as the data layer holds it. By default a custom validation can't run
    /// atomically.
    fn atomic(&self, _ctx: &crate::atomic::AtomicContext<'_>) -> crate::atomic::Atomic {
        crate::atomic::Atomic::not_atomic("a custom validation without an atomic implementation")
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ArgumentDef {
    pub name: &'static str,
    pub ty: AttrType,
    pub allow_nil: bool,
    /// Its value when input doesn't give it, as Ash's argument `default`.
    pub default: Option<fn() -> crate::value::Value>,
}

impl ArgumentDef {
    pub const fn new(name: &'static str, ty: AttrType) -> Self {
        Self {
            name,
            ty,
            allow_nil: false,
            default: None,
        }
    }

    pub const fn optional(name: &'static str, ty: AttrType) -> Self {
        Self {
            name,
            ty,
            allow_nil: true,
            default: None,
        }
    }

    /// This argument, `default` when input doesn't give it.
    pub const fn with_default(mut self, default: fn() -> crate::value::Value) -> Self {
        self.default = Some(default);
        self
    }
}

/// `arguments` with each default an argument `input` doesn't give, as Ash fills them in.
pub fn apply_argument_defaults(arguments: &[ArgumentDef], input: &mut crate::value::FieldMap) {
    for arg in arguments {
        if let Some(default) = arg.default
            && !input.contains_key(arg.name)
        {
            input.insert(arg.name.to_string(), default());
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Create,
    Read,
    Update,
    Destroy,
    Generic,
}

impl ActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Read => "read",
            Self::Update => "update",
            Self::Destroy => "destroy",
            Self::Generic => "generic",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistKind {
    DataLayer,
    Manual,
}

#[derive(Clone, Copy)]
pub enum PreparationDef {
    Filter(fn() -> crate::filter::Filter),
    FilterWithArgs(fn(&crate::value::FieldMap) -> crate::filter::Filter),
    Sort {
        field: &'static str,
        descending: bool,
    },
    Limit(usize),
    Offset(usize),
    /// Runs on the records the read found, with the read's arguments, as Ash's
    /// `prepare after_action(...)`: to note metadata on them, say.
    AfterAction(AfterReadFn),
}

/// What a read's `after_action` preparation runs on the records it found.
pub type AfterReadFn = fn(&crate::value::FieldMap, &mut [crate::value::FieldMap]) -> Result<()>;

impl std::fmt::Debug for PreparationDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Filter(_) => f.write_str("PreparationDef::Filter(..)"),
            Self::FilterWithArgs(_) => f.write_str("PreparationDef::FilterWithArgs(..)"),
            Self::Sort { field, descending } => f
                .debug_struct("Sort")
                .field("field", field)
                .field("descending", descending)
                .finish(),
            Self::Limit(n) => f.debug_tuple("Limit").field(n).finish(),
            Self::Offset(n) => f.debug_tuple("Offset").field(n).finish(),
            Self::AfterAction(_) => f.write_str("PreparationDef::AfterAction(..)"),
        }
    }
}

impl PreparationDef {
    pub const fn filter(f: fn() -> crate::filter::Filter) -> Self {
        Self::Filter(f)
    }

    pub const fn filter_with_args(f: fn(&crate::value::FieldMap) -> crate::filter::Filter) -> Self {
        Self::FilterWithArgs(f)
    }

    pub const fn sort(field: &'static str, descending: bool) -> Self {
        Self::Sort { field, descending }
    }

    pub const fn limit(limit: usize) -> Self {
        Self::Limit(limit)
    }

    pub const fn offset(offset: usize) -> Self {
        Self::Offset(offset)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ActionDef {
    pub name: &'static str,
    pub kind: ActionKind,
    pub accept: &'static [&'static str],
    pub arguments: &'static [ArgumentDef],
    pub primary: bool,
    pub changes: &'static [Change],
    pub validations: &'static [Validation],
    pub preparations: &'static [PreparationDef],
    pub persist: PersistKind,
    /// A soft destroy stores the changed record instead of deleting it, and skips
    /// `on_delete` cascades because the row stays.
    pub soft: bool,
    /// Relationships whose related records are destroyed first, with their primary
    /// destroy action.
    pub cascade_destroy: &'static [&'static str],
    /// For an update or a soft destroy: run as one statement in the data layer, or fail,
    /// as Ash's `require_atomic?` (true by default). One whose changes or validations
    /// can't run in the data layer must set this to false to read the record first
    /// instead. A hard destroy runs as one statement when it can, and reads first when it
    /// can't, whatever this says, as in Ash.
    pub require_atomic: bool,
    /// For an update or destroy run atomically: the read action whose filters decide
    /// which records it reaches, as Ash's `atomic_upgrade_with`; the primary read when
    /// `None`.
    pub atomic_upgrade_with: Option<&'static str>,
    /// For a read: how it pages, as Ash's `pagination`. `None`: it doesn't.
    pub pagination: Option<Pagination>,
    /// For a generic action: what it returns, as Ash's `returns`; `None` when nothing.
    pub returns: Option<AttrType>,
    /// For a generic action: whether it runs in a transaction, as Ash's `transaction?`.
    pub transaction: bool,
    /// What the action may note on the records it answers, beyond their fields, as Ash's
    /// action `metadata` (see [`crate::put_metadata`]).
    pub metadata: &'static [MetadataDef],
}

/// Something an action notes on a record it answers, as Ash's `metadata :name, :type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetadataDef {
    pub name: &'static str,
    pub ty: AttrType,
    pub allow_nil: bool,
}

impl MetadataDef {
    pub const fn new(name: &'static str, ty: AttrType) -> Self {
        Self { name, ty, allow_nil: true }
    }

    pub const fn required(name: &'static str, ty: AttrType) -> Self {
        Self { name, ty, allow_nil: false }
    }
}

/// How a read action pages, as Ash's `pagination` declares it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pagination {
    /// Pages after or before a record's keyset.
    pub keyset: bool,
    /// Pages by offset.
    pub offset: bool,
    /// Whether a page may count every record its read finds.
    pub countable: Countable,
    /// The page size when a page doesn't give one.
    pub default_limit: Option<usize>,
    /// The largest page; a larger limit is cut to it. Ash's default is 250.
    pub max_page_size: Option<usize>,
    /// Whether every read pages (with the default limit when none is given). Ash's
    /// default.
    pub required: bool,
}

/// Whether a page may count its read's records, as Ash's `countable`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Countable {
    No,
    /// When the page asks.
    Yes,
    /// Unless the page asks not to.
    ByDefault,
}

impl Pagination {
    /// Keyset pages, with Ash's defaults otherwise.
    pub const fn keyset() -> Self {
        Self {
            keyset: true,
            offset: false,
            countable: Countable::No,
            default_limit: None,
            max_page_size: Some(250),
            required: true,
        }
    }

    /// Offset pages, with Ash's defaults otherwise.
    pub const fn offset() -> Self {
        Self { keyset: false, offset: true, ..Self::keyset() }
    }

    /// Offset pages as well.
    pub const fn and_offset(mut self) -> Self {
        self.offset = true;
        self
    }

    pub const fn countable(mut self, countable: Countable) -> Self {
        self.countable = countable;
        self
    }

    pub const fn default_limit(mut self, limit: usize) -> Self {
        self.default_limit = Some(limit);
        self
    }

    pub const fn max_page_size(mut self, max: Option<usize>) -> Self {
        self.max_page_size = max;
        self
    }

    pub const fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }
}

impl ActionDef {
    pub const fn create(name: &'static str) -> Self {
        Self {
            name,
            kind: ActionKind::Create,
            accept: &[],
            arguments: &[],
            primary: false,
            changes: &[],
            validations: &[],
            preparations: &[],
            persist: PersistKind::DataLayer,
            soft: false,
            cascade_destroy: &[],
            require_atomic: true,
            atomic_upgrade_with: None,
            pagination: None,
            returns: None,
            transaction: false,
            metadata: &[],
        }
    }

    pub const fn read(name: &'static str) -> Self {
        Self {
            name,
            kind: ActionKind::Read,
            accept: &[],
            arguments: &[],
            primary: false,
            changes: &[],
            validations: &[],
            preparations: &[],
            persist: PersistKind::DataLayer,
            soft: false,
            cascade_destroy: &[],
            require_atomic: true,
            atomic_upgrade_with: None,
            pagination: None,
            returns: None,
            transaction: false,
            metadata: &[],
        }
    }

    pub const fn update(name: &'static str) -> Self {
        Self {
            name,
            kind: ActionKind::Update,
            accept: &[],
            arguments: &[],
            primary: false,
            changes: &[],
            validations: &[],
            preparations: &[],
            persist: PersistKind::DataLayer,
            soft: false,
            cascade_destroy: &[],
            require_atomic: true,
            atomic_upgrade_with: None,
            pagination: None,
            returns: None,
            transaction: false,
            metadata: &[],
        }
    }

    pub const fn destroy(name: &'static str) -> Self {
        Self {
            name,
            kind: ActionKind::Destroy,
            accept: &[],
            arguments: &[],
            primary: false,
            changes: &[],
            validations: &[],
            preparations: &[],
            persist: PersistKind::DataLayer,
            soft: false,
            cascade_destroy: &[],
            require_atomic: true,
            atomic_upgrade_with: None,
            pagination: None,
            returns: None,
            transaction: false,
            metadata: &[],
        }
    }

    pub const fn generic(name: &'static str) -> Self {
        Self {
            name,
            kind: ActionKind::Generic,
            accept: &[],
            arguments: &[],
            primary: false,
            changes: &[],
            validations: &[],
            preparations: &[],
            persist: PersistKind::DataLayer,
            soft: false,
            cascade_destroy: &[],
            require_atomic: true,
            atomic_upgrade_with: None,
            pagination: None,
            returns: None,
            transaction: false,
            metadata: &[],
        }
    }

    pub const fn accept(mut self, accept: &'static [&'static str]) -> Self {
        self.accept = accept;
        self
    }

    pub const fn arguments(mut self, arguments: &'static [ArgumentDef]) -> Self {
        self.arguments = arguments;
        self
    }

    pub fn argument(&self, name: &str) -> Option<&ArgumentDef> {
        self.arguments.iter().find(|arg| arg.name == name)
    }

    pub fn has_argument(&self, name: &str) -> bool {
        self.argument(name).is_some()
    }

    pub const fn changes(mut self, changes: &'static [Change]) -> Self {
        self.changes = changes;
        self
    }

    pub const fn validations(mut self, validations: &'static [Validation]) -> Self {
        self.validations = validations;
        self
    }

    pub const fn preparations(mut self, preparations: &'static [PreparationDef]) -> Self {
        self.preparations = preparations;
        self
    }

    pub const fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    /// The read action an atomic update reaches records through: see
    /// [`atomic_upgrade_with`](Self::atomic_upgrade_with).
    pub const fn atomic_upgrade_with(mut self, read: &'static str) -> Self {
        self.atomic_upgrade_with = Some(read);
        self
    }

    /// What a generic action returns, as Ash's `returns`.
    pub const fn returns(mut self, ty: AttrType) -> Self {
        self.returns = Some(ty);
        self
    }

    /// Whether a generic action runs in a transaction, as Ash's `transaction?`.
    pub const fn transaction(mut self, transaction: bool) -> Self {
        self.transaction = transaction;
        self
    }

    /// How a read pages, as Ash's `pagination`.
    pub const fn metadata(mut self, metadata: &'static [MetadataDef]) -> Self {
        self.metadata = metadata;
        self
    }

    pub const fn pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = Some(pagination);
        self
    }

    /// Whether an update or soft destroy must run atomically, as Ash's `require_atomic?`.
    pub const fn require_atomic(mut self, require: bool) -> Self {
        self.require_atomic = require;
        self
    }

    pub const fn manual(mut self) -> Self {
        self.persist = PersistKind::Manual;
        self
    }

    pub const fn soft(mut self) -> Self {
        self.soft = true;
        self
    }

    pub const fn cascade_destroy(mut self, relationships: &'static [&'static str]) -> Self {
        self.cascade_destroy = relationships;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ManagedRelType {
    #[default]
    Create,
    DirectControl,
    Append,
}

#[derive(Clone, Copy)]
pub enum Change {
    SetAttribute {
        field: &'static str,
        value: ConstValue,
    },
    SetNewAttribute {
        field: &'static str,
        value: ConstValue,
    },
    RelateActor {
        field: &'static str,
    },
    SetFromArgument {
        field: &'static str,
        argument: &'static str,
    },
    /// Runtime `set` of a non-literal value (enum variant, path, etc.).
    SetAttributeFn {
        field: &'static str,
        value: fn() -> crate::value::Value,
    },
    /// Runtime `set_new` of a non-literal value.
    SetNewAttributeFn {
        field: &'static str,
        value: fn() -> crate::value::Value,
    },
    ManageRelationship {
        relationship: &'static str,
        rel_type: ManagedRelType,
    },
    /// Sets `field` to `expr` over the record as stored, as Ash's
    /// `atomic_update(:field, expr(...))`: in the update's statement where it can run as
    /// one, or computed from the record read first.
    AtomicUpdate {
        field: &'static str,
        expr: &'static crate::expr::Expr,
    },
    BeforeAction(BeforeActionFn),
    AfterAction(AfterActionFn),
    AfterTransaction(AfterTransactionFn),
    Custom(&'static dyn CustomChange),
    Func(fn(&mut ChangeContext<'_>) -> Result<()>),
}

impl Change {
    pub const fn before_action(f: BeforeActionFn) -> Self {
        Self::BeforeAction(f)
    }

    pub const fn after_action(f: AfterActionFn) -> Self {
        Self::AfterAction(f)
    }

    pub const fn after_transaction(f: AfterTransactionFn) -> Self {
        Self::AfterTransaction(f)
    }

    pub const fn manage_relationship(relationship: &'static str, rel_type: ManagedRelType) -> Self {
        Self::ManageRelationship {
            relationship,
            rel_type,
        }
    }
}

impl std::fmt::Debug for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SetAttribute { field, value } => f
                .debug_struct("SetAttribute")
                .field("field", field)
                .field("value", value)
                .finish(),
            Self::SetNewAttribute { field, value } => f
                .debug_struct("SetNewAttribute")
                .field("field", field)
                .field("value", value)
                .finish(),
            Self::RelateActor { field } => {
                f.debug_struct("RelateActor").field("field", field).finish()
            }
            Self::SetFromArgument { field, argument } => f
                .debug_struct("SetFromArgument")
                .field("field", field)
                .field("argument", argument)
                .finish(),
            Self::SetAttributeFn { field, .. } => f
                .debug_struct("SetAttributeFn")
                .field("field", field)
                .finish(),
            Self::SetNewAttributeFn { field, .. } => f
                .debug_struct("SetNewAttributeFn")
                .field("field", field)
                .finish(),
            Self::ManageRelationship {
                relationship,
                rel_type,
            } => f
                .debug_struct("ManageRelationship")
                .field("relationship", relationship)
                .field("rel_type", rel_type)
                .finish(),
            Self::AtomicUpdate { field, .. } => f.debug_struct("AtomicUpdate").field("field", field).finish(),
            Self::BeforeAction(_) => write!(f, "BeforeAction(<fn>)"),
            Self::AfterAction(_) => write!(f, "AfterAction(<fn>)"),
            Self::AfterTransaction(_) => write!(f, "AfterTransaction(<fn>)"),
            Self::Custom(_) => write!(f, "Custom(<dyn CustomChange>)"),
            Self::Func(_) => write!(f, "Func(<fn>)"),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Validation {
    Present {
        field: &'static str,
    },
    StringLength {
        field: &'static str,
        min: Option<usize>,
        max: Option<usize>,
    },
    OneOf {
        field: &'static str,
        allowed: &'static [&'static str],
    },
    Numericality {
        field: &'static str,
        min: Option<i64>,
        max: Option<i64>,
    },
    Custom(&'static dyn CustomValidation),
    Func(fn(&ValidationContext<'_>) -> Result<()>),
}

impl Validation {
    /// The error a built-in validation fails with, as Ash describes it: its message a
    /// template, with its vars (`must have length of between %{min} and %{max}`). `None`
    /// for a custom validation, which gives its own.
    pub fn error(&self) -> Option<Error> {
        use crate::value::Value;
        let text = |text: &str| Value::String(text.to_string());
        let number = |n: Option<i64>| n.map_or(Value::Null, Value::Int);
        Some(match *self {
            Self::Present { field } => Error::validation(
                field,
                "must be present",
                vec![
                    ("attributes".into(), Value::Array(vec![text(field)])),
                    ("exactly".into(), Value::Int(1)),
                    ("fields".into(), Value::Array(vec![text(field)])),
                    ("keys".into(), text(field)),
                ],
            ),
            Self::StringLength { field, min, max } => {
                let (message, vars) = match (min, max) {
                    (Some(min), Some(max)) => (
                        "must have length of between %{min} and %{max}",
                        vec![("min".into(), Value::Int(min as i64)), ("max".into(), Value::Int(max as i64))],
                    ),
                    (Some(min), None) => ("must have length of at least %{min}", vec![("min".into(), Value::Int(min as i64))]),
                    (None, Some(max)) => ("must have length of no more than %{max}", vec![("max".into(), Value::Int(max as i64))]),
                    (None, None) => return None,
                };
                Error::validation(field, message, vars)
            }
            Self::OneOf { field, allowed } => {
                Error::validation(field, "expected one of %{values}", vec![("values".into(), text(&allowed.join(", ")))])
            }
            // As Ash's `compare`, with bounds it may and may not be at.
            Self::Numericality { field, min, max } => {
                let mut parts = Vec::new();
                if min.is_some() {
                    parts.push("must be greater than or equal to %{greater_than_or_equal_to}");
                }
                if max.is_some() {
                    parts.push("must be less than or equal to %{less_than_or_equal_to}");
                }
                Error::validation(
                    field,
                    parts.join(" and "),
                    vec![
                        ("greater_than".into(), Value::Null),
                        ("less_than".into(), Value::Null),
                        ("greater_than_or_equal_to".into(), number(min)),
                        ("less_than_or_equal_to".into(), number(max)),
                        ("is_equal".into(), Value::Null),
                        ("is_not_equal".into(), Value::Null),
                        ("is_nil".into(), Value::Null),
                    ],
                )
            }
            Self::Custom(_) | Self::Func(_) => return None,
        })
    }
}

impl std::fmt::Debug for Validation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Present { field } => f.debug_struct("Present").field("field", field).finish(),
            Self::StringLength { field, min, max } => f
                .debug_struct("StringLength")
                .field("field", field)
                .field("min", min)
                .field("max", max)
                .finish(),
            Self::OneOf { field, allowed } => f
                .debug_struct("OneOf")
                .field("field", field)
                .field("allowed", allowed)
                .finish(),
            Self::Numericality { field, min, max } => f
                .debug_struct("Numericality")
                .field("field", field)
                .field("min", min)
                .field("max", max)
                .finish(),
            Self::Custom(_) => write!(f, "Custom(<dyn CustomValidation>)"),
            Self::Func(_) => write!(f, "Func(<fn>)"),
        }
    }
}

impl Validation {
    pub const fn present(field: &'static str) -> Self {
        Self::Present { field }
    }

    pub const fn string_length(
        field: &'static str,
        min: Option<usize>,
        max: Option<usize>,
    ) -> Self {
        Self::StringLength { field, min, max }
    }

    pub const fn one_of(field: &'static str, allowed: &'static [&'static str]) -> Self {
        Self::OneOf { field, allowed }
    }

    pub const fn numericality(field: &'static str, min: Option<i64>, max: Option<i64>) -> Self {
        Self::Numericality { field, min, max }
    }

    pub const fn custom(c: &'static dyn CustomValidation) -> Self {
        Self::Custom(c)
    }

    pub const fn func(f: fn(&ValidationContext<'_>) -> Result<()>) -> Self {
        Self::Func(f)
    }
}

impl Change {
    pub const fn custom(c: &'static dyn CustomChange) -> Self {
        Self::Custom(c)
    }

    pub const fn func(f: fn(&mut ChangeContext<'_>) -> Result<()>) -> Self {
        Self::Func(f)
    }
}
