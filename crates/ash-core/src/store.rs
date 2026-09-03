//! Type-safe store marker tags and compile-time presence verification.
//!
//! In `ash-rust`, data layer binding can be explicitly specified at the type level
//! via [`StoreTag`]. This enables multiple distinct instances of the same storage
//! engine (e.g. `PrimarySqlite` vs `AuditSqlite`), third-party storage backends,
//! and compile-time guarantees.

use crate::registry::DynDataLayer;

/// A marker trait identifying a logical database or storage target.
pub trait StoreTag: 'static + Send + Sync {}

/// The default store marker used when a resource does not specify an explicit store.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DefaultStore;
impl StoreTag for DefaultStore {}

/// Built-in convenience store tag for SQLite-backed storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SqliteStore;
impl StoreTag for SqliteStore {}

/// Built-in convenience store tag for in-memory storage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct MemoryStore;
impl StoreTag for MemoryStore {}

/// Trait implemented by contexts or registries that provide storage for a specific [`StoreTag`].
pub trait HasStore<T: StoreTag> {
    /// Retrieve the type-erased data layer for store `T`.
    fn get_store(&self) -> &dyn DynDataLayer;
}

/// Helper function returning the `TypeId` of [`DefaultStore`].
pub fn default_store_type_id() -> std::any::TypeId {
    std::any::TypeId::of::<DefaultStore>()
}
