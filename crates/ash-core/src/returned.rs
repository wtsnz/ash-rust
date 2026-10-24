//! What a generic action returns, as a value, for an API that runs it by name: an Ash
//! type's value, else what it serializes to. The macros call it; it's not for code to.

use crate::error::{Error, Result};
use crate::types::AshType;
use crate::value::Value;

type ProjectValue<'a> = dyn Fn(crate::AttrType, Value) -> Result<crate::PresentedValue> + 'a;
type OutputProjector<T> = fn(T, &ProjectValue<'_>) -> Result<ActionResult<T>>;

/// An action's typed result and its reader-specific presentation, kept together until
/// the typed caller or response adapter chooses its output representation.
pub struct ActionResult<T> {
    value: T,
    presented: Option<crate::PresentedValue>,
}

impl<T> ActionResult<T> {
    pub fn into_inner(self) -> T {
        self.value
    }
    pub fn into_parts(self) -> (T, Option<crate::PresentedValue>) {
        (self.value, self.presented)
    }
}

/// A generic action result with its presentation contract. Macro-generated runners
/// retain the Rust type so embedded resources cannot be mistaken for opaque maps.
pub struct ActionOutput<T> {
    value: T,
    project: OutputProjector<T>,
}

impl<T> ActionOutput<T> {
    pub(crate) fn present<D>(self, ctx: &crate::Context<D>) -> Result<ActionResult<T>> {
        (self.project)(self.value, &|ty, value| ctx.present_value(ty, value))
    }
}

/// Holds the result while autoref dispatch selects its presentation contract.
pub struct PendingOutput<T>(std::cell::RefCell<Option<T>>);

impl<T> PendingOutput<T> {
    pub fn new(value: T) -> Self {
        Self(std::cell::RefCell::new(Some(value)))
    }
}

pub trait AshOutput<T> {
    fn output(self) -> ActionOutput<T>;
}

impl<T: AshType> AshOutput<T> for &&PendingOutput<T> {
    fn output(self) -> ActionOutput<T> {
        ActionOutput {
            value: self
                .0
                .borrow_mut()
                .take()
                .expect("action result taken once"),
            project: |value, project| {
                let presented = project(T::ATTR_TYPE, value.to_value())?;
                let value = T::from_value(&presented.to_value())?;
                Ok(ActionResult {
                    value,
                    presented: Some(presented),
                })
            },
        }
    }
}

/// Opaque Rust results have no Ash field metadata. Their serializer controls their
/// application-defined representation. Resource-bearing results must implement
/// `AshType`, for example through `AshTypedMap`, to receive field presentation.
pub trait OpaqueOutput<T> {
    fn output(self) -> ActionOutput<T>;
}

impl<T> OpaqueOutput<T> for &PendingOutput<T> {
    fn output(self) -> ActionOutput<T> {
        ActionOutput {
            value: self
                .0
                .borrow_mut()
                .take()
                .expect("action result taken once"),
            project: |value, _| {
                Ok(ActionResult {
                    value,
                    presented: None,
                })
            },
        }
    }
}

/// A generic action's result, waiting to become a value.
pub struct Returned<T>(pub T);

/// An Ash type's own value.
pub trait AsAshValue {
    fn value(&self) -> Result<Value>;
}

impl<T: AshType> AsAshValue for &&Returned<T> {
    fn value(&self) -> Result<Value> {
        Ok(self.0.to_value())
    }
}

/// Anything serializable, as the JSON it serializes to.
pub trait AsSerializedValue {
    fn value(&self) -> Result<Value>;
}

impl<T: serde::Serialize> AsSerializedValue for &Returned<T> {
    fn value(&self) -> Result<Value> {
        serde_json::to_value(&self.0)
            .map(Value::from_plain_json)
            .map_err(|e| Error::Invalid(e.to_string()))
    }
}

/// Anything else, which has no value.
pub trait NoValue {
    fn value(&self) -> Result<Value>;
}

impl<T> NoValue for Returned<T> {
    fn value(&self) -> Result<Value> {
        Err(Error::Invalid(format!(
            "a generic action returning {} can't be run by name: it's neither an Ash type nor serializable",
            std::any::type_name::<T>()
        )))
    }
}

/// Type metadata for a generic return, resolved after macro expansion so resource
/// types and typed composites keep their field definitions.
pub struct ReturnType<T>(pub std::marker::PhantomData<fn() -> T>);

pub trait AshReturnType {
    fn return_type(self) -> crate::AttrType;
}
impl<T: AshType> AshReturnType for &&ReturnType<T> {
    fn return_type(self) -> crate::AttrType {
        T::ATTR_TYPE
    }
}
pub trait OpaqueReturnType {
    fn return_type(self) -> crate::AttrType;
}
impl<T> OpaqueReturnType for &ReturnType<T> {
    fn return_type(self) -> crate::AttrType {
        crate::AttrType::Map
    }
}
