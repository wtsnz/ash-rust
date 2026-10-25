//! Runtime-owned authenticated encryption for pagination tokens.

use crate::{Context, Error, FieldMap, KeysetCursor, ResourceDef, Result, Sort, Value};
use base64::prelude::*;
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, AeadCore, OsRng, Payload},
};

pub(crate) struct CursorKey(XChaCha20Poly1305);

impl std::fmt::Debug for CursorKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CursorKey([redacted])")
    }
}

impl CursorKey {
    pub(crate) fn random() -> Self {
        Self(XChaCha20Poly1305::new(&XChaCha20Poly1305::generate_key(
            &mut OsRng,
        )))
    }
    pub(crate) fn new(key: [u8; 32]) -> Self {
        Self(XChaCha20Poly1305::new(&key.into()))
    }
}

fn invalid() -> Error {
    Error::Invalid("invalid or expired keyset cursor".into())
}

impl<D> Context<D> {
    fn cursor_binding(
        &self,
        resource: &ResourceDef,
        action: &str,
        sorts: &[Sort],
        distinct: Option<&crate::DistinctOn>,
    ) -> Result<Vec<u8>> {
        let shape: Vec<_> = sorts
            .iter()
            .map(|s| {
                (
                    &s.field,
                    s.descending,
                    s.nulls_first(),
                    s.projected,
                    format!("{:?}", s.guard),
                )
            })
            .collect();
        // A token cannot change the meaning of a different read, reader or tenant.
        let mut binding = serde_json::to_value((
            "ash-keyset-v1",
            resource.name,
            action,
            self.tenant(),
            self.actor().map(crate::Actor::cursor_identity),
            self.authorization_actor()
                .is_some_and(crate::Actor::is_system),
            shape,
        ))
        .map_err(|_| invalid())?;
        if let Some(distinct) = distinct {
            binding = serde_json::json!([
                "ash-distinct-keyset-v1",
                binding,
                distinct.fields,
                format!("{:?}", distinct.sort)
            ]);
        }
        binding.sort_all_objects();
        serde_json::to_vec(&binding).map_err(|_| invalid())
    }

    /// Seal the actual values used by the effective sort, before result projection.
    pub fn seal_keyset(
        &self,
        resource: &ResourceDef,
        action: &str,
        sorts: &[Sort],
        fields: &FieldMap,
    ) -> Result<String> {
        self.seal_keyset_with_distinct(resource, action, sorts, None, fields)
    }

    /// Seal a page cursor bound to its distinct groups and winner ordering too.
    pub fn seal_keyset_with_distinct(
        &self,
        resource: &ResourceDef,
        action: &str,
        sorts: &[Sort],
        distinct: Option<&crate::DistinctOn>,
        fields: &FieldMap,
    ) -> Result<String> {
        let cursor = KeysetCursor {
            id: resource
                .primary_key()
                .and_then(|pk| fields.get(pk.name))
                .cloned()
                .unwrap_or(Value::Null),
            values: sorts
                .iter()
                .map(|sort| {
                    let value = if sort
                        .guard
                        .as_ref()
                        .is_some_and(|guard| !guard.matches_on(resource, fields))
                    {
                        Value::Null
                    } else {
                        crate::derived::sort_value(fields, sort)
                            .cloned()
                            .unwrap_or(Value::Null)
                    };
                    (sort.field.clone(), value)
                })
                .collect(),
        };
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let plaintext = serde_json::to_vec(&cursor).map_err(|_| invalid())?;
        let aad = self.cursor_binding(resource, action, sorts, distinct)?;
        let ciphertext = self
            .scope
            .cursor_key
            .0
            .encrypt(
                &nonce,
                Payload {
                    msg: &plaintext,
                    aad: &aad,
                },
            )
            .map_err(|_| invalid())?;
        let mut token = nonce.to_vec();
        token.extend(ciphertext);
        Ok(format!("v1.{}", BASE64_URL_SAFE_NO_PAD.encode(token)))
    }

    /// Only authenticated server cursors supply trusted comparison values. Legacy JSON,
    /// base64 JSON and bare identifiers are rejected at this network boundary.
    fn open_keyset(
        &self,
        resource: &ResourceDef,
        action: &str,
        sorts: &[Sort],
        token: &str,
        distinct: Option<&crate::DistinctOn>,
    ) -> Result<KeysetCursor> {
        let bytes = BASE64_URL_SAFE_NO_PAD
            .decode(token.strip_prefix("v1.").ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        if bytes.len() < 40 {
            return Err(invalid());
        }
        let aad = self.cursor_binding(resource, action, sorts, distinct)?;
        let plaintext = self
            .scope
            .cursor_key
            .0
            .decrypt(
                XNonce::from_slice(&bytes[..24]),
                Payload {
                    msg: &bytes[24..],
                    aad: &aad,
                },
            )
            .map_err(|_| invalid())?;
        let cursor: KeysetCursor = serde_json::from_slice(&plaintext).map_err(|_| invalid())?;
        if cursor.values.len() != sorts.len()
            || !cursor
                .values
                .iter()
                .zip(sorts)
                .all(|((field, _), sort)| field == &sort.field)
        {
            return Err(invalid());
        }
        Ok(cursor)
    }

    pub fn keyset_filter(
        &self,
        resource: &ResourceDef,
        action: &str,
        sorts: &[Sort],
        token: &str,
        after: bool,
    ) -> Result<crate::Filter> {
        self.keyset_filter_with_distinct(resource, action, sorts, None, token, after)
    }

    /// Decode a page boundary using the same distinct plan that selected its winner.
    pub fn keyset_filter_with_distinct(
        &self,
        resource: &ResourceDef,
        action: &str,
        sorts: &[Sort],
        distinct: Option<&crate::DistinctOn>,
        token: &str,
        after: bool,
    ) -> Result<crate::Filter> {
        let cursor = self.open_keyset(resource, action, sorts, token, distinct)?;
        let filter = crate::build_keyset_filter(
            resource,
            sorts,
            &crate::keyset_values(resource, &cursor, sorts),
            after,
        )
        .ok_or_else(invalid)?;
        crate::plan_filter(
            resource,
            self.authorization_actor(),
            self.tenant(),
            crate::TrustedExpr::Filter(filter),
        )
        .map(|filter| crate::Filter::Keyset(Box::new(filter)))
    }
}
