use std::time::{SystemTime, UNIX_EPOCH};

use ash_core::{Context, DataLayer, Filter, Value};
use uuid::Uuid;

use crate::error::{AuthError, Result};

ash_core::resource! {
    resource AshToken;
    table "ash_tokens";

    attributes {
        id: Uuid [pk],
        jti: String,
        subject: Uuid,
        purpose: String,
        expires_at: i64,
        revoked: bool = false,
        extra: Option<String>,
    }

    actions {
        create store {
            primary;
            accept [jti, subject, purpose, expires_at, revoked, extra];
        }

        read read {
            primary;
        }

        update revoke {
            primary;
            change set_attribute(revoked, true);
        }

        destroy destroy {
            primary;
        }
    }
}

/// Persistent token store backed by an Ash `DataLayer` (e.g. SQLite, PostgreSQL, Memory).
#[derive(Clone)]
pub struct DatabaseTokenStore<D: DataLayer> {
    context: Context<D>,
}

impl<D: DataLayer> DatabaseTokenStore<D> {
    /// Create a new `DatabaseTokenStore` for a given context.
    pub fn new(context: Context<D>) -> Self {
        Self { context }
    }

    /// Access underlying context.
    pub fn context(&self) -> &Context<D> {
        &self.context
    }

    /// Current timestamp in seconds since Unix epoch.
    fn now_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Store a new token in the database.
    pub async fn store_token(
        &self,
        jti: impl Into<String>,
        subject: Uuid,
        purpose: impl Into<String>,
        expires_at: i64,
        extra: Option<String>,
    ) -> Result<AshToken> {
        let jti_str = jti.into();
        let purpose_str = purpose.into();

        AshToken::store(&self.context)
            .jti(jti_str)
            .subject(subject)
            .purpose(purpose_str)
            .expires_at(expires_at)
            .revoked(false)
            .extra(extra)
            .await
            .map_err(AuthError::Core)
    }

    /// Find an active token by `jti` and `purpose`.
    ///
    /// Returns `None` if token does not exist or has expired.
    pub async fn get_valid_token(&self, jti: &str, purpose: &str) -> Result<Option<AshToken>> {
        let now = Self::now_secs();
        let filter = Filter::eq("jti", Value::String(jti.to_string()))
            & Filter::eq("purpose", Value::String(purpose.to_string()))
            & Filter::eq("revoked", Value::Bool(false))
            & Filter::gt("expires_at", Value::Int(now));

        let tokens = AshToken::query(&self.context)
            .filter(filter)
            .limit(1)
            .all()
            .await
            .map_err(AuthError::Core)?;

        Ok(tokens.into_iter().next())
    }

    /// Check whether a `jti` has been marked as revoked in the database.
    pub async fn is_revoked(&self, jti: &str) -> Result<bool> {
        let filter = Filter::eq("jti", Value::String(jti.to_string()))
            & (Filter::eq("revoked", Value::Bool(true))
                | Filter::eq("purpose", Value::String("revocation".to_string())));

        let tokens = AshToken::query(&self.context)
            .filter(filter)
            .limit(1)
            .all()
            .await
            .map_err(AuthError::Core)?;

        Ok(!tokens.is_empty())
    }

    /// Mark an existing token as revoked, or insert a revocation tombstone record.
    pub async fn revoke_token(
        &self,
        jti: &str,
        subject: Option<Uuid>,
        expires_at: i64,
    ) -> Result<()> {
        // 1. Look for existing token with this jti
        let filter = Filter::eq("jti", Value::String(jti.to_string()));
        let existing = AshToken::query(&self.context)
            .filter(filter)
            .all()
            .await
            .map_err(AuthError::Core)?;

        if existing.is_empty() {
            // Insert a revocation record
            let sub = subject.unwrap_or_else(Uuid::nil);
            self.store_token(jti, sub, "revocation", expires_at, None)
                .await?;
        } else {
            for token in existing {
                if !token.revoked {
                    token.revoke_on(&self.context).await.map_err(AuthError::Core)?;
                }
            }
        }

        Ok(())
    }

    /// Revoke all active refresh tokens for a given subject (user ID).
    pub async fn revoke_all_for_subject(&self, subject: Uuid) -> Result<usize> {
        let filter = Filter::eq("subject", Value::Uuid(subject))
            & Filter::eq("revoked", Value::Bool(false));

        let tokens = AshToken::query(&self.context)
            .filter(filter)
            .all()
            .await
            .map_err(AuthError::Core)?;

        let count = tokens.len();
        for token in tokens {
            token.revoke_on(&self.context).await.map_err(AuthError::Core)?;
        }

        Ok(count)
    }

    /// Prune all expired tokens from the database.
    pub async fn prune_expired(&self) -> Result<usize> {
        let now = Self::now_secs();
        let filter = Filter::lt("expires_at", Value::Int(now));

        let expired = AshToken::query(&self.context)
            .filter(filter)
            .all()
            .await
            .map_err(AuthError::Core)?;

        let count = expired.len();
        for token in expired {
            token.destroy_on(&self.context).await.map_err(AuthError::Core)?;
        }

        Ok(count)
    }
}
