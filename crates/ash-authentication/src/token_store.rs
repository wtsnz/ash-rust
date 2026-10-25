use std::time::{SystemTime, UNIX_EPOCH};

use ash_core::{Context, CredentialWritePermit, DataLayer, FieldMap, Filter, Value};
use uuid::Uuid;

use crate::error::{AuthError, Result};
use crate::token::Claims;

ash_core::resource! {
    AshToken {
        table "ash_tokens";
        attributes {
            id: Uuid [pk];
            resource: String;
            tenant_key: String;
            jti: String;
            subject: Uuid;
            purpose: String;
            expires_at: i64;
            revoked: bool = false;
            consumed: bool = false;
            extra: Option<String>;
        }
        identities { identity token_namespace [resource, tenant_key, subject, jti]; }
        actions {
            create store {
                primary;
                accept [resource, tenant_key, jti, subject, purpose, expires_at, revoked, consumed, extra];
            }
            read read { primary; }
            update revoke { primary; change set(revoked, true); }
            destroy destroy { primary; }
        }
    }
}

/// Persisted session state. Construct with a Runtime-issued write grant for AshToken.
/// All operations retain the subject resource and exact optional tenant namespace.
#[derive(Clone)]
pub struct DatabaseTokenStore<D: DataLayer> {
    context: Context<D>,
    resource: &'static str,
    write: CredentialWritePermit,
}

impl<D: DataLayer> DatabaseTokenStore<D> {
    pub fn new(context: Context<D>, resource: &'static str, write: CredentialWritePermit) -> Self {
        Self {
            context,
            resource,
            write,
        }
    }

    pub fn context(&self) -> &Context<D> {
        &self.context
    }

    fn now_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before Unix epoch")
            .as_secs() as i64
    }

    fn tenant_key(&self) -> String {
        // JSON encodes None as null, distinct from every tenant string, including "null".
        serde_json::to_string(&self.context.tenant()).expect("optional string serialization")
    }

    fn namespace(&self) -> Filter {
        Filter::eq("resource", self.resource) & Filter::eq("tenant_key", self.tenant_key())
    }

    fn token_filter(&self, claims: &Claims) -> Result<Filter> {
        self.context
            .access()
            .require_tenant_match(claims.tenant.as_deref())?;
        if claims.resource != self.resource {
            return Err(AuthError::InvalidToken("wrong resource".into()));
        }
        Ok(self.namespace()
            & Filter::eq("subject", claims.sub)
            & Filter::eq("jti", claims.jti.clone())
            & Filter::eq("purpose", claims.purpose.clone()))
    }

    pub async fn store_token(&self, claims: &Claims) -> Result<AshToken> {
        self.token_filter(claims)?;
        AshToken::store(&self.context)
            .resource(self.resource)
            .tenant_key(self.tenant_key())
            .jti(claims.jti.clone())
            .subject(claims.sub)
            .purpose(claims.purpose.clone())
            .expires_at(claims.exp as i64)
            .revoked(false)
            .consumed(false)
            .extra(None)
            .await
            .map_err(AuthError::Core)
    }

    /// Missing state fails closed, including tokens signed outside the strategy.
    pub async fn get_valid_token(&self, claims: &Claims) -> Result<Option<AshToken>> {
        let filter = self.token_filter(claims)?
            & Filter::eq("revoked", false)
            & Filter::eq("consumed", false)
            & Filter::gt("expires_at", Self::now_secs());
        Ok(AshToken::query(&self.context)
            .filter(filter)
            .limit(1)
            .all()
            .await?
            .into_iter()
            .next())
    }

    /// Atomic compare-and-set. Exactly one row is success; there is no read-before-write.
    pub async fn consume(&self, claims: &Claims) -> Result<()> {
        let filter = self.token_filter(claims)?
            & Filter::eq("revoked", false)
            & Filter::eq("consumed", false)
            & Filter::gt("expires_at", Self::now_secs());
        let fields = FieldMap::from([("consumed".into(), Value::Bool(true))]);
        let rows = self
            .context
            .access()
            .update_credentials_where(&self.write, filter, fields)
            .await?;
        if rows.len() != 1 {
            return Err(AuthError::TokenRevoked);
        }
        Ok(())
    }

    pub async fn revoke_token(&self, claims: &Claims) -> Result<()> {
        self.revoke_filter(self.token_filter(claims)?).await?;
        Ok(())
    }

    /// Server-side JTI revocation within this resource and tenant. Public input policy
    /// and retention are separate from durable storage.
    pub async fn revoke_jti(&self, jti: &str) -> Result<()> {
        let filter = self.namespace() & Filter::eq("jti", jti);
        if self.revoke_filter(filter).await? == 0 {
            AshToken::store(&self.context)
                .resource(self.resource)
                .tenant_key(self.tenant_key())
                .jti(jti)
                .subject(Uuid::nil())
                .purpose("revocation")
                .expires_at(i64::MAX)
                .revoked(true)
                .consumed(false)
                .extra(None)
                .await?;
        }
        Ok(())
    }

    async fn revoke_filter(&self, filter: Filter) -> Result<usize> {
        let fields = FieldMap::from([("revoked".into(), Value::Bool(true))]);
        Ok(self
            .context
            .access()
            .update_credentials_where(&self.write, filter, fields)
            .await?
            .len())
    }

    pub async fn revoke_all_for_subject(&self, subject: Uuid) -> Result<usize> {
        self.revoke_filter(
            self.namespace() & Filter::eq("subject", subject) & Filter::eq("revoked", false),
        )
        .await
    }

    pub async fn prune_expired(&self) -> Result<usize> {
        let expired = AshToken::query(&self.context)
            .filter(self.namespace() & Filter::lt("expires_at", Self::now_secs()))
            .all()
            .await?;
        let count = expired.len();
        for token in expired {
            token.destroy_on(&self.context).await?;
        }
        Ok(count)
    }
}
