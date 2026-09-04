use ash_core::{Actor, Context, DataLayer, FieldMap, Filter, Resource, ResourceDef, Value};
use ash_core::Result as CoreResult;
use uuid::Uuid;

use crate::api_key::ApiKeyService;
use crate::def::{AuthenticationDef, PasswordStrategyDef};
use crate::error::{AuthError, Result};
use crate::password::PasswordService;
use crate::token::{JwtService, TokenPair};
use crate::token_store::DatabaseTokenStore;

/// High-level authentication strategy coordinator.
#[derive(Clone)]
pub struct AuthStrategy<R: Resource> {
    resource_def: &'static ResourceDef,
    password_service: PasswordService,
    api_key_service: ApiKeyService,
    jwt_service: Option<JwtService>,
    _phantom: std::marker::PhantomData<R>,
}

impl<R: Resource> AuthStrategy<R> {
    /// Create an `AuthStrategy` for an authenticable resource.
    pub fn new(resource_def: &'static ResourceDef) -> Self {
        Self {
            resource_def,
            password_service: PasswordService::new(),
            api_key_service: ApiKeyService::default(),
            jwt_service: None,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Attach a JWT service for token generation and verification.
    pub fn with_jwt_service(mut self, jwt: JwtService) -> Self {
        self.jwt_service = Some(jwt);
        self
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
            .data
            .run_query(
                self.resource_def,
                &ash_core::CompiledQuery {
                    filter: Some(filter),
                    limit: Some(1),
                    ..ash_core::CompiledQuery::default()
                },
            )
            .await
            .map_err(AuthError::Core)?;

        let record_fields = records
            .into_iter()
            .next()
            .ok_or(AuthError::UserNotFound)?;

        R::from_fields(&record_fields).map_err(AuthError::Core)
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
        let auth_def = self.config().cloned().unwrap_or_default();
        let pass_cfg = auth_def
            .password
            .unwrap_or_else(PasswordStrategyDef::default);

        let identity = identity_val.into();

        // 1. Query for the record by identity field
        let filter = Filter::eq(pass_cfg.identity_field, identity);
        let records: Vec<R> = ctx
            .data
            .run_query(
                self.resource_def,
                &ash_core::CompiledQuery {
                    filter: Some(filter),
                    limit: Some(2),
                    ..ash_core::CompiledQuery::default()
                },
            )
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

        Ok(record)
    }

    /// Authenticate a user by identity and password, and automatically generate a signed JWT bearer token.
    pub async fn sign_in_with_password_and_token<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        identity_val: impl Into<Value>,
        password: &str,
    ) -> Result<(R, String)> {
        let record = self
            .sign_in_with_password(ctx, identity_val, password)
            .await?;
        let user_id = record.id();

        let jwt = self.jwt_service.as_ref().ok_or_else(|| {
            AuthError::Crypto("JwtService not configured on AuthStrategy".to_string())
        })?;

        // Extract role if record has a "role" field
        let fields = record.to_fields();
        let role = match fields.get("role") {
            Some(Value::String(r)) => Some(r.clone()),
            _ => None,
        };

        let token = jwt.sign_token(user_id, role, ctx.tenant.clone())?;
        Ok((record, token))
    }

    /// Authenticate a user by identity and password, issuing an `(access_token, refresh_token)` pair,
    /// and persisting the refresh token in the database token store.
    pub async fn sign_in_with_password_and_token_pair<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        identity_val: impl Into<Value>,
        password: &str,
    ) -> Result<(R, TokenPair)> {
        let record = self
            .sign_in_with_password(ctx, identity_val, password)
            .await?;
        let user_id = record.id();

        let jwt = self.jwt_service.as_ref().ok_or_else(|| {
            AuthError::Crypto("JwtService not configured on AuthStrategy".to_string())
        })?;

        let fields = record.to_fields();
        let role = match fields.get("role") {
            Some(Value::String(r)) => Some(r.clone()),
            _ => None,
        };

        let pair = jwt.sign_token_pair(user_id, role, ctx.tenant.clone())?;

        // Persist refresh token in database
        let refresh_claims = jwt.verify_token_with_purpose(&pair.refresh_token, "refresh")?;
        let token_store = DatabaseTokenStore::new(ctx.clone());
        token_store
            .store_token(
                &refresh_claims.jti,
                user_id,
                "refresh",
                refresh_claims.exp as i64,
                None,
            )
            .await?;

        Ok((record, pair))
    }

    /// Rotate an active refresh token, issuing a new token pair and revoking the old refresh token.
    pub async fn rotate_refresh_token<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        refresh_token: &str,
    ) -> Result<(R, TokenPair)> {
        let jwt = self.jwt_service.as_ref().ok_or_else(|| {
            AuthError::Crypto("JwtService not configured on AuthStrategy".to_string())
        })?;

        let claims = jwt.verify_token_with_purpose(refresh_token, "refresh")?;
        let token_store = DatabaseTokenStore::new(ctx.clone());

        // Verify token exists and is not revoked
        let valid_token = token_store
            .get_valid_token(&claims.jti, "refresh")
            .await?
            .ok_or_else(|| {
                AuthError::InvalidToken(
                    "refresh token does not exist or has already been used".to_string(),
                )
            })?;

        // Invalidate old refresh token (Token Rotation!)
        valid_token.revoke_on(ctx).await.map_err(AuthError::Core)?;

        // Fetch user
        let user = self.get_user_by_id(ctx, claims.sub).await?;

        let fields = user.to_fields();
        let role = match fields.get("role") {
            Some(Value::String(r)) => Some(r.clone()),
            _ => None,
        };

        // Issue new token pair
        let pair = jwt.sign_token_pair(user.id(), role, ctx.tenant.clone())?;

        // Store new refresh token
        let new_refresh_claims = jwt.verify_token_with_purpose(&pair.refresh_token, "refresh")?;
        token_store
            .store_token(
                &new_refresh_claims.jti,
                user.id(),
                "refresh",
                new_refresh_claims.exp as i64,
                None,
            )
            .await?;

        Ok((user, pair))
    }

    /// Revoke a token (by either JWT string or raw identifier).
    pub async fn revoke_token<D: DataLayer>(&self, ctx: &Context<D>, token_or_jti: &str) -> Result<()> {
        let jwt = self.jwt_service.as_ref();
        let token_store = DatabaseTokenStore::new(ctx.clone());

        // Check if input is a valid JWT or a raw JTI
        if let Some(jwt) = jwt
            && let Ok(claims) = jwt.verify_token(token_or_jti)
        {
            token_store
                .revoke_token(&claims.jti, Some(claims.sub), claims.exp as i64)
                .await?;
            let _ = jwt.revoke_jti(&claims.jti, claims.exp);
        } else {
            token_store
                .revoke_token(token_or_jti, None, i64::MAX)
                .await?;
            if let Some(jwt) = jwt {
                let _ = jwt.revoke_jti(token_or_jti, u64::MAX);
            }
        }

        Ok(())
    }

    /// Change a user's password with current password verification.
    ///
    /// Revokes all active refresh tokens for the user to ensure any compromised sessions are terminated.
    pub async fn change_password<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        user_id: Uuid,
        current_password: &str,
        new_password: &str,
        new_password_confirmation: &str,
    ) -> Result<R> {
        let auth_def = self.config().cloned().unwrap_or_default();
        let pass_cfg = auth_def
            .password
            .unwrap_or_else(PasswordStrategyDef::default);

        // 1. Load user
        let user = self.get_user_by_id(ctx, user_id).await?;

        // 2. Verify current password
        let fields = user.to_fields();
        let hashed_password = match fields.get(pass_cfg.hashed_password_field) {
            Some(Value::String(s)) => s.as_str(),
            _ => return Err(AuthError::InvalidCredentials),
        };

        if !self
            .password_service
            .verify_password(current_password, hashed_password)?
        {
            return Err(AuthError::InvalidCredentials);
        }

        // 3. Verify confirmation
        if new_password != new_password_confirmation {
            return Err(AuthError::PasswordConfirmationMismatch);
        }

        // 4. Hash new password
        let new_hash = self
            .password_service
            .clone()
            .with_min_length(pass_cfg.min_password_length)
            .hash_password(new_password)?;

        // 5. Update user
        let mut update_fields = FieldMap::new();
        update_fields.insert(
            pass_cfg.hashed_password_field.to_string(),
            Value::String(new_hash),
        );

        let updated_raw = ctx
            .data
            .update(self.resource_def, user_id, update_fields)
            .await
            .map_err(AuthError::Core)?;

        let updated_user = R::from_fields(&updated_raw).map_err(AuthError::Core)?;

        // 6. Revoke active refresh tokens for this subject
        let token_store = DatabaseTokenStore::new(ctx.clone());
        let _ = token_store.revoke_all_for_subject(user_id).await;

        Ok(updated_user)
    }

    /// Initiate a password reset flow, generating a single-use reset token.
    pub async fn request_password_reset<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        email: &str,
    ) -> Result<(R, String)> {
        let auth_def = self.config().cloned().unwrap_or_default();
        let pass_cfg = auth_def
            .password
            .unwrap_or_else(PasswordStrategyDef::default);

        let jwt = self.jwt_service.as_ref().ok_or_else(|| {
            AuthError::Crypto("JwtService not configured on AuthStrategy".to_string())
        })?;

        // 1. Look up user by identity (email)
        let filter = Filter::eq(pass_cfg.identity_field, Value::String(email.to_string()));
        let records = ctx
            .data
            .run_query(
                self.resource_def,
                &ash_core::CompiledQuery {
                    filter: Some(filter),
                    limit: Some(1),
                    ..ash_core::CompiledQuery::default()
                },
            )
            .await
            .map_err(AuthError::Core)?;

        let user_raw = records.into_iter().next().ok_or(AuthError::UserNotFound)?;
        let user = R::from_fields(&user_raw).map_err(AuthError::Core)?;

        // 2. Generate reset token
        let reset_token = jwt.sign_reset_token(user.id(), ctx.tenant.clone())?;

        // 3. Store reset token in DatabaseTokenStore
        let claims = jwt.verify_token_with_purpose(&reset_token, "password_reset")?;
        let token_store = DatabaseTokenStore::new(ctx.clone());
        token_store
            .store_token(
                &claims.jti,
                user.id(),
                "password_reset",
                claims.exp as i64,
                None,
            )
            .await?;

        Ok((user, reset_token))
    }

    /// Complete a password reset flow using a single-use reset token.
    pub async fn reset_password_with_token<D: DataLayer>(
        &self,
        ctx: &Context<D>,
        reset_token: &str,
        new_password: &str,
        new_password_confirmation: &str,
    ) -> Result<R> {
        let auth_def = self.config().cloned().unwrap_or_default();
        let pass_cfg = auth_def
            .password
            .unwrap_or_else(PasswordStrategyDef::default);

        let jwt = self.jwt_service.as_ref().ok_or_else(|| {
            AuthError::Crypto("JwtService not configured on AuthStrategy".to_string())
        })?;

        // 1. Verify reset token
        let claims = jwt.verify_token_with_purpose(reset_token, "password_reset")?;
        let token_store = DatabaseTokenStore::new(ctx.clone());

        // 2. Ensure reset token is active and not already consumed
        let valid_token = token_store
            .get_valid_token(&claims.jti, "password_reset")
            .await?
            .ok_or_else(|| {
                AuthError::InvalidToken("reset token not found or already consumed".to_string())
            })?;

        // 3. Validate confirmation
        if new_password != new_password_confirmation {
            return Err(AuthError::PasswordConfirmationMismatch);
        }

        // 4. Hash new password
        let new_hash = self
            .password_service
            .clone()
            .with_min_length(pass_cfg.min_password_length)
            .hash_password(new_password)?;

        // 5. Update user
        let mut update_fields = FieldMap::new();
        update_fields.insert(
            pass_cfg.hashed_password_field.to_string(),
            Value::String(new_hash),
        );

        let updated_raw = ctx
            .data
            .update(self.resource_def, claims.sub, update_fields)
            .await
            .map_err(AuthError::Core)?;

        let updated_user = R::from_fields(&updated_raw).map_err(AuthError::Core)?;

        // 6. Invalidate the single-use reset token
        valid_token.revoke_on(ctx).await.map_err(AuthError::Core)?;

        // 7. Revoke active refresh tokens for security
        let _ = token_store.revoke_all_for_subject(claims.sub).await;

        Ok(updated_user)
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

        let hash = self.api_key_service.hash_api_key(raw_api_key);

        let filter = Filter::eq(api_key_cfg.api_key_field, Value::String(hash));
        let records: Vec<R> = ctx
            .data
            .run_query(
                self.resource_def,
                &ash_core::CompiledQuery {
                    filter: Some(filter),
                    limit: Some(1),
                    ..ash_core::CompiledQuery::default()
                },
            )
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

    /// Convert an authenticated record into an `ash_core::Actor` ready to be placed on `Context.actor`.
    pub fn to_actor(&self, record: &R) -> Actor {
        let mut actor = Actor::new(record.id());
        for (k, v) in record.to_fields() {
            // Do not copy password hashes into the actor metadata
            if !k.contains("password") && !k.contains("hash") {
                actor = actor.with(k, v);
            }
        }
        actor
    }
}
