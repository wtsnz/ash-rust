//! Resource-level credential versioning, applied within each backend write.

use crate::{AtomicExpr, AtomicUpdate, FieldMap, ResourceDef, Value};

/// A password hash and the version invalidating JWT sessions when that hash changes.
#[derive(Clone, Copy, Debug)]
pub struct CredentialVersionRule {
    pub hash_field: &'static str,
    pub version_field: &'static str,
}

impl CredentialVersionRule {
    pub fn for_resource(resource: &ResourceDef) -> Option<Self> {
        resource
            .extensions
            .iter()
            .find_map(|extension| extension.credential_version_rule())
    }

    /// Initialize a new subject. Caller-provided versions never select session state.
    pub fn initialize(resource: &ResourceDef, fields: &mut FieldMap) {
        if let Some(rule) = Self::for_resource(resource) {
            fields.insert(rule.version_field.into(), Value::Int(0));
        }
    }

    /// Replace any supplied version with a change relative to the stored hash/version.
    /// Backends must evaluate this expression against the row under their write lock.
    pub fn apply(resource: &ResourceDef, update: &mut AtomicUpdate) {
        let Some(rule) = Self::for_resource(resource) else {
            return;
        };
        let supplied_version = update
            .set
            .iter()
            .any(|(name, _)| name == rule.version_field);
        update.set.retain(|(name, _)| name != rule.version_field);
        if let Some((_, hash)) = update.set.iter().find(|(name, _)| name == rule.hash_field) {
            let hash = hash.clone();
            update.set(
                rule.version_field,
                AtomicExpr::If {
                    condition: Box::new(AtomicExpr::DistinctFrom(
                        Box::new(AtomicExpr::field(rule.hash_field)),
                        Box::new(hash),
                    )),
                    then: Box::new(AtomicExpr::Add(
                        Box::new(AtomicExpr::field(rule.version_field)),
                        Box::new(AtomicExpr::value(1i64)),
                    )),
                    otherwise: Box::new(AtomicExpr::field(rule.version_field)),
                },
            );
        } else if supplied_version {
            update.set(rule.version_field, AtomicExpr::field(rule.version_field));
        }
    }

    pub fn update(resource: &ResourceDef, fields: &FieldMap) -> AtomicUpdate {
        let mut update = AtomicUpdate::default();
        for (name, value) in fields {
            update.set(name.clone(), AtomicExpr::Value(value.clone()));
        }
        Self::apply(resource, &mut update);
        update
    }
}
