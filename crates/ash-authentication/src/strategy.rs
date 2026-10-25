use std::sync::Arc;

use ash_core::Result as CoreResult;
use ash_core::{
    Actor, Context, CredentialReadPermit, DataLayer, FieldMap, Filter, Resource, ResourceDef, Value,
};
use uuid::Uuid;

use crate::api_key::ApiKeyService;
use crate::def::{AuthenticationDef, PasswordStrategyDef};
use crate::error::{AuthError, Result};
use crate::password::PasswordService;
use crate::sender::{AuthSender, NoopSender};
use crate::token::{Claims, JwtService, TokenPair};
use crate::token_store::AshToken;
use crate::token_store::DatabaseTokenStore;

/// High-level authentication strategy coordinator.
///
/// The server adapter must select the tenant before calling this strategy. All
/// credential lookups retain that tenant, including reads that bypass user policies.
#[derive(Clone)]
pub struct AuthStrategy<R: Resource> {
    resource_def: &'static ResourceDef,
    credential_read: CredentialReadPermit,
    credential_write: ash_core::CredentialWritePermit,
    token_write: ash_core::CredentialWritePermit,
    password_service: PasswordService,
    api_key_service: ApiKeyService,
    jwt_service: Option<JwtService>,
    sender: Arc<dyn AuthSender>,
    _phantom: std::marker::PhantomData<R>,
}

impl<R: Resource> AuthStrategy<R> {
    /// Create an `AuthStrategy` for an authenticable resource.
    pub fn new<D>(runtime: &ash_core::Runtime<D>) -> Self {
        let resource_def = &R::DEF;
        let credential_read = runtime.credential_reads::<R>();
        let fields = R::DEF
            .extensions
            .iter()
            .find_map(|extension| extension.as_any().downcast_ref::<AuthenticationDef>())
            .and_then(|auth| auth.password.as_ref())
            .map(|password| password.hashed_password_field);
        let credential_write = runtime.credential_writes::<R>(fields);
        let api_key_service = resource_def
            .extension::<AuthenticationDef>()
            .and_then(|auth| auth.api_key.as_ref())
            .map(|key| ApiKeyService::new(key.key_prefix))
            .unwrap_or_default();
        Self {
            resource_def,
            credential_read,
            credential_write,
            token_write: runtime.credential_writes::<AshToken>(["revoked", "consumed"]),
            password_service: PasswordService::new(),
            api_key_service,
            jwt_service: None,
            sender: Arc::new(NoopSender),
            _phantom: std::marker::PhantomData,
        }
    }

    /// Attach a JWT service for token generation and verification.
    pub fn with_jwt_service(mut self, jwt: JwtService) -> Self {
        self.jwt_service = Some(jwt);
        self
    }

    /// Attach an authentication sender for dispatching reset/confirmation tokens.
    pub fn with_sender(mut self, sender: impl AuthSender) -> Self {
        self.sender = Arc::new(sender);
        self
    }

    /// Attach a shared authentication sender.
    pub fn with_arc_sender(mut self, sender: Arc<dyn AuthSender>) -> Self {
        self.sender = sender;
        self
    }

    /// Reference to the configured authentication sender.
    pub fn sender(&self) -> &Arc<dyn AuthSender> {
        &self.sender
    }

    /// Reference to JWT service if configured.
    pub fn jwt_service(&self) -> Option<&JwtService> {
        self.jwt_service.as_ref()
    }

    /// Reference to password service.
    pub fn password_service(&self) -> &PasswordService {
        &self.password_service
    }

    /// Reference to API key service.
    pub fn api_key_service(&self) -> &ApiKeyService {
        &self.api_key_service
    }

    /// Read the `AuthenticationDef` from the resource extensions if configured.
    pub fn config(&self) -> Option<&AuthenticationDef> {
        self.resource_def.extension::<AuthenticationDef>()
    }

    /// Helper to find a user record by primary key.
    pub async fn get_user_by_id<D: DataLayer>(&self, ctx: &Context<D>, id: Uuid) -> Result<R> {
        let pk_field = self
            .resource_def
            .primary_key()
            .map(|a| a.name)
            .unwrap_or("id");

        let filter = Filter::eq(pk_field, Value::Uuid(id));
        let records = ctx
            .access()
            .credentials(&self.credential_read, filter, 1)
            .await
            .map_err(AuthError::Core)?;

        let record_fields = records.into_iter().next().ok_or(AuthError::UserNotFound)?;

        R::from_fields(&record_fields).map_err(AuthError::Core)
    }

    fn password_config(&self) -> Result<&PasswordStrategyDef> {
        let invalid = || {
            AuthError::Core(ash_core::Error::Invalid(
            "password authentication requires AuthenticationDef and its credential version rule".into()
        ))
        };
        let password = self
            .config()
            .and_then(|auth| auth.password.as_ref())
            .ok_or_else(invalid)?;
        let rule =
            ash_core::CredentialVersionRule::for_resource(self.resource_def).ok_or_else(invalid)?;
        if rule.hash_field != password.hashed_password_field
            || rule.version_field != "credential_version"
            || !self
                .resource_def
                .attribute(rule.version_field)
                .is_some_and(|attribute| {
                    attribute.ty == ash_core::AttrType::Integer && !attribute.primary_key
                })
        {
            return Err(invalid());
        }
        Ok(password)
    }

    fn jwt(&self) -> Result<&JwtService> {
        self.jwt_service
            .as_ref()
            .ok_or_else(|| AuthError::Crypto("JwtService not configured on AuthStrategy".into()))
    }

    pub fn token_store<D: DataLayer>(&self, ctx: &Context<D>) -> DatabaseTokenStore<D> {
        DatabaseTokenStore::new(
            ctx.clone(),
            self.resource_def.name,
            self.token_write.clone(),
        )
    }

    fn version(&self, user: &R) -> Result<i64> {
        let config = self.config().ok_or_else(|| {
            AuthError::Core(ash_core::Error::Invalid(
                "JWT authentication requires AuthenticationDef".into(),
            ))
        })?;
        if config.password.is_some() {
            self.password_config()?;
        }
        match user.to_fields().get("credential_version") {
            Some(Value::Int(version)) if *version >= 0 => Ok(*version),
            _ => Err(AuthError::InvalidToken(
                "subject has no credential version".into(),
            )),
        }
    }

    fn signed_pair<D>(&self, ctx: &Context<D>, user: &R) -> Result<TokenPair> {
        let fields = user.to_fields();
        let role = fields
            .get("role")
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.jwt()?.sign_token_pair(
            user.id(),
            role,
            ctx.tenant().map(str::to_owned),
            self.resource_def.name,
            self.version(user)?,
        )
    }

    async fn persist_pair<D: DataLayer>(&self, ctx: &Context<D>, pair: &TokenPair) -> Result<()> {
        let store = self.token_store(ctx);
        for token in [&pair.access_token, &pair.refresh_token] {
            store.store_token(&self.jwt()?.verify_token(token)?).await?;
        }
        Ok(())
    }

    async fn live_subject<D: DataLayer>(&self, ctx: &Context<D>, claims: &Claims) -> Result<R> {
        ctx.access()
            .require_tenant_match(claims.tenant.as_deref())?;
        if claims.resource != self.resource_def.name {
            return Err(AuthError::InvalidToken("wrong resource".into()));
        }
        let user = self.get_user_by_id(ctx, claims.sub).await?;
        if claims.credential_version != self.version(&user)? {
            return Err(AuthError::TokenRevoked);
        }
        Ok(user)
    }

    /// Authenticate a request against persisted session state and a live subject.
    /// The caller resolves the tenant before this call. No accepted-session cache is used.
    pub async fn authenticate<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        token: &str,
    ) -> Result<crate::BoundIdentity> {
        let claims = self.jwt()?.verify_token_with_purpose(token, "access")?;
        ctx.required_transaction(|tx| async move {
            let user = self.live_subject(&tx, &claims).await?;
            if self
                .token_store(&tx)
                .get_valid_token(&claims)
                .await?
                .is_none()
            {
                return Err(AuthError::TokenRevoked.into());
            }
            Ok(crate::BoundIdentity::new(
                self.to_actor(&user),
                claims.tenant,
            ))
        })
        .await
        .map_err(AuthError::Core)
    }

    /// Authenticate a user by identity (email/username) and plaintext password.
    ///
    /// Performs constant-time verification and returns the authenticated record `R`.
    pub async fn sign_in_with_password<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        identity_val: impl Into<Value>,
        password: &str,
    ) -> Result<R> {
        let pass_cfg = self.password_config()?;
        let auth_def = self.config().expect("validated authentication config");

        let identity = identity_val.into();

        // 1. Query for the record by identity field
        let filter = Filter::eq(pass_cfg.identity_field, identity);
        let records: Vec<R> = ctx
            .access()
            .credentials(&self.credential_read, filter, 2)
            .await
            .map_err(AuthError::Core)?
            .into_iter()
            .map(|f| R::from_fields(&f))
            .collect::<CoreResult<Vec<R>>>()
            .map_err(AuthError::Core)?;

        let record = match records.into_iter().next() {
            Some(r) => r,
            None => return Err(AuthError::InvalidCredentials),
        };

        // 2. Extract hashed password
        let fields = record.to_fields();
        let hashed_password = match fields.get(pass_cfg.hashed_password_field) {
            Some(Value::String(s)) => s.as_str(),
            _ => return Err(AuthError::InvalidCredentials),
        };

        // 3. Verify password
        let valid = self
            .password_service
            .verify_password(password, hashed_password)?;
        if !valid {
            return Err(AuthError::InvalidCredentials);
        }

        if let Some(confirmation) = &auth_def.confirmation
            && confirmation.prevent_unconfirmed_sign_in
        {
            let confirmed = match fields.get(confirmation.confirmed_field) {
                None | Some(Value::Null) => false,
                Some(Value::Bool(false)) => false,
                Some(Value::String(s)) if s.trim().is_empty() => false,
                Some(_) => true,
            };
            if !confirmed {
                return Err(AuthError::UserUnconfirmed);
            }
        }

        Ok(record)
    }

    /// Authenticate credentials and persist a new access session.
    pub async fn sign_in_with_password_and_token<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        identity_val: impl Into<Value>,
        password: &str,
    ) -> Result<(R, String)> {
        let user = self
            .sign_in_with_password(ctx, identity_val, password)
            .await?;
        let fields = user.to_fields();
        let role = fields
            .get("role")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let token = self.jwt()?.sign_access_token(
            user.id(),
            role,
            ctx.tenant().map(str::to_owned),
            self.resource_def.name,
            self.version(&user)?,
        )?;
        let claims = self.jwt()?.verify_token_with_purpose(&token, "access")?;
        ctx.required_transaction(|tx| async move {
            self.token_store(&tx).store_token(&claims).await?;
            Ok(())
        })
        .await?;
        Ok((user, token))
    }

    /// Issue and persist an access/refresh pair in one required transaction.
    pub async fn sign_in_with_password_and_token_pair<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        identity_val: impl Into<Value>,
        password: &str,
    ) -> Result<(R, TokenPair)> {
        let user = self
            .sign_in_with_password(ctx, identity_val, password)
            .await?;
        let pair = self.signed_pair(ctx, &user)?;
        let pair_to_store = &pair;
        ctx.required_transaction(|tx| async move {
            self.persist_pair(&tx, pair_to_store)
                .await
                .map_err(ash_core::Error::from)
        })
        .await?;
        Ok((user, pair))
    }

    /// Consume a refresh token exactly once and persist its successor in the same transaction.
    pub async fn rotate_refresh_token<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        token: &str,
    ) -> Result<(R, TokenPair)> {
        let claims = self.jwt()?.verify_token_with_purpose(token, "refresh")?;
        ctx.access()
            .require_tenant_match(claims.tenant.as_deref())?;
        ctx.required_transaction(|tx| async move {
            self.token_store(&tx).consume(&claims).await?;
            let user = self.live_subject(&tx, &claims).await?;
            let pair = self.signed_pair(&tx, &user)?;
            self.persist_pair(&tx, &pair).await?;
            Ok((user, pair))
        })
        .await
        .map_err(AuthError::Core)
    }

    /// Revoke a verified JWT in the resolved resource/tenant namespace. Possession
    /// authorizes revoking only that token. Repeated revocation is idempotent.
    pub async fn revoke_token<D: DataLayer>(&self, ctx: &Context<D>, token: &str) -> Result<()> {
        let claims = self.jwt()?.verify_token(token)?;
        self.token_store(ctx).revoke_token(&claims).await
    }

    /// Trusted server-only revocation of existing JTI state. No signer is needed,
    /// and unknown JTIs create no rows. Never forward untrusted HTTP input here.
    pub async fn revoke_jti<D: DataLayer>(&self, ctx: &Context<D>, jti: &str) -> Result<()> {
        self.token_store(ctx).revoke_jti(jti).await
    }

    /// Authenticate an access token and revoke only its owner's sessions in the
    /// resolved resource/tenant namespace, within one required transaction.
    pub async fn revoke_own_sessions<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        access_token: &str,
    ) -> Result<()> {
        ctx.required_transaction(|tx| async move {
            let identity = self.authenticate(&tx, access_token).await?;
            self.token_store(&tx)
                .revoke_all_for_subject(identity.actor().id)
                .await?;
            Ok(())
        })
        .await
        .map_err(AuthError::Core)
    }

    /// Validate/hash before the write transaction. The stored hash comparison prevents
    /// another password change from authorizing this write with a stale password.
    pub async fn change_password<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        user_id: Uuid,
        current_password: &str,
        new_password: &str,
        new_password_confirmation: &str,
    ) -> Result<R> {
        let pass_cfg = self.password_config()?;
        let user = self.get_user_by_id(ctx, user_id).await?;
        let fields = user.to_fields();
        let old_hash = fields
            .get(pass_cfg.hashed_password_field)
            .and_then(Value::as_str)
            .ok_or(AuthError::InvalidCredentials)?;
        if !self
            .password_service
            .verify_password(current_password, old_hash)?
        {
            return Err(AuthError::InvalidCredentials);
        }
        let new_hash = self.hash_new_password(new_password, new_password_confirmation)?;
        ctx.required_transaction(|tx| async move {
            let pk = self
                .resource_def
                .primary_key()
                .ok_or(ash_core::Error::NoPrimaryKey(self.resource_def.name))?
                .name;
            let filter =
                Filter::eq(pk, user_id) & Filter::eq(pass_cfg.hashed_password_field, old_hash);
            let changes = FieldMap::from([(
                pass_cfg.hashed_password_field.into(),
                Value::String(new_hash),
            )]);
            let mut rows = tx
                .access()
                .update_credentials_where(&self.credential_write, filter, changes)
                .await?;
            if rows.len() != 1 {
                return Err(AuthError::InvalidCredentials.into());
            }
            let user = R::from_fields(rows.pop().expect("one updated subject").fields())?;
            self.token_store(&tx)
                .revoke_all_for_subject(user_id)
                .await?;
            Ok(user)
        })
        .await
        .map_err(AuthError::Core)
    }

    fn hash_new_password(&self, password: &str, confirmation: &str) -> Result<String> {
        if password != confirmation {
            return Err(AuthError::PasswordConfirmationMismatch);
        }
        let min = self.password_config()?.min_password_length;
        self.password_service
            .clone()
            .with_min_length(min)
            .hash_password(password)
    }

    /// Persist a reset token, commit, then dispatch external mail. This entry point
    /// owns commit and rejects an outer transaction which could still roll back mail.
    pub async fn request_password_reset<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        email: &str,
    ) -> Result<(R, String)> {
        if ctx.has_transaction_boundary() {
            return Err(AuthError::Core(ash_core::Error::Invalid(
                "password reset mail requires an outermost commit boundary".into(),
            )));
        }
        let pass_cfg = self.password_config()?;
        let user_raw = ctx
            .access()
            .credentials(
                &self.credential_read,
                Filter::eq(pass_cfg.identity_field, email),
                1,
            )
            .await?
            .into_iter()
            .next();
        let user = user_raw.as_ref().map(R::from_fields).transpose()?;
        // Unknown identities do the same signing and verification work. No
        // password is supplied here, so neither path performs password hashing.
        let (subject, version) = match &user {
            Some(user) => (user.id(), self.version(user)?),
            None => (Uuid::new_v4(), 0),
        };
        let token = self.jwt()?.sign_reset_token(
            subject,
            ctx.tenant().map(str::to_owned),
            self.resource_def.name,
            version,
        )?;
        let claims = self
            .jwt()?
            .verify_token_with_purpose(&token, "password_reset")?;
        let user = user.ok_or(AuthError::UserNotFound)?;
        ctx.required_transaction(|tx| async move {
            self.token_store(&tx).store_token(&claims).await?;
            Ok(())
        })
        .await?;
        self.sender
            .send_password_reset(&user.to_fields(), &token)
            .await?;
        Ok((user, token))
    }

    /// Hash/validate first; atomically consume, update hash/version and revoke all
    /// prior JWT sessions. Failure rolls back consumption along with subject state.
    pub async fn reset_password_with_token<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        token: &str,
        password: &str,
        confirmation: &str,
    ) -> Result<R> {
        let claims = self
            .jwt()?
            .verify_token_with_purpose(token, "password_reset")?;
        ctx.access()
            .require_tenant_match(claims.tenant.as_deref())?;
        let hash = self.hash_new_password(password, confirmation)?;
        let pass_cfg = self.password_config()?;
        ctx.required_transaction(|tx| async move {
            self.token_store(&tx).consume(&claims).await?;
            self.live_subject(&tx, &claims).await?;
            let changes =
                FieldMap::from([(pass_cfg.hashed_password_field.into(), Value::String(hash))]);
            let pk = self
                .resource_def
                .primary_key()
                .ok_or(ash_core::Error::NoPrimaryKey(self.resource_def.name))?
                .name;
            let filter = Filter::eq(pk, claims.sub)
                & Filter::eq("credential_version", claims.credential_version);
            let mut rows = tx
                .access()
                .update_credentials_where(&self.credential_write, filter, changes)
                .await?;
            if rows.len() != 1 {
                return Err(AuthError::TokenRevoked.into());
            }
            let user = R::from_fields(rows.pop().expect("one updated subject").fields())?;
            self.token_store(&tx)
                .revoke_all_for_subject(claims.sub)
                .await?;
            Ok(user)
        })
        .await
        .map_err(AuthError::Core)
    }

    /// Authenticate a user by an API key.
    pub async fn authenticate_api_key<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        raw_api_key: &str,
    ) -> Result<R> {
        let auth_def = self.config().cloned().unwrap_or_default();
        let api_key_cfg = auth_def
            .api_key
            .unwrap_or_else(crate::def::ApiKeyStrategyDef::default);

        self.api_key_service.validate_key(raw_api_key)?;
        let hash = self.api_key_service.hash_api_key(raw_api_key);

        let filter = Filter::eq(api_key_cfg.api_key_field, Value::String(hash));
        let records: Vec<R> = ctx
            .access()
            .credentials(&self.credential_read, filter, 1)
            .await
            .map_err(AuthError::Core)?
            .into_iter()
            .map(|f| R::from_fields(&f))
            .collect::<CoreResult<Vec<R>>>()
            .map_err(AuthError::Core)?;

        records
            .into_iter()
            .next()
            .ok_or(AuthError::InvalidCredentials)
    }

    /// Convert an authenticated record into an `ash_core::Actor` ready for `Runtime::bind`.
    pub fn to_actor(&self, record: &R) -> Actor {
        let mut actor = Actor::new(record.id());
        for (k, v) in record.to_fields() {
            // Do not copy password hashes into the actor metadata
            if !self.resource_def.sensitive_attribute(&k)
                && !k.contains("password")
                && !k.contains("hash")
            {
                actor = actor.with(k, v);
            }
        }
        actor
    }
}
