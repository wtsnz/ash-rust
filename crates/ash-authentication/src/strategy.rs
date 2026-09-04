use ash_core::{Actor, Context, DataLayer, Filter, Resource, ResourceDef, Value};
use ash_core::Result as CoreResult;

use crate::api_key::ApiKeyService;
use crate::def::{AuthenticationDef, PasswordStrategyDef};
use crate::error::{AuthError, Result};
use crate::password::PasswordService;
use crate::token::JwtService;

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

    /// Read the `AuthenticationDef` from the resource extensions if configured.
    pub fn config(&self) -> Option<&AuthenticationDef> {
        self.resource_def.extension::<AuthenticationDef>()
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
