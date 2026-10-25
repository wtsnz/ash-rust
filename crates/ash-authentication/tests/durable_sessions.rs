use std::sync::Arc;

use ash_authentication::{AshToken, AuthStrategy, Claims, JwtService, authentication};
use ash_core::{Context, DataLayer, FieldMap, Resource, Runtime, TransactionSupport};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

const SECRET: &str = "durable-session-tests-secret-key-0123456789";
const PASSWORD: &str = "original-password-123";
const NEW_PASSWORD: &str = "replacement-password-456";
const EMAIL: &str = "member@example.test";

#[authentication]
ash_core::resource! {
    User {
        table "session_users";
        multitenancy { strategy: attribute; attribute: org; }
        attributes {
            id: Uuid [pk];
            org: String;
            email: String;
        }
        authentication {
            strategy password { identity_field: email; }
            strategy tokens { token_lifetime_secs: 3600; }
            strategy api_key { api_key_field: api_key_hash; }
        }
        identities { identity unique_email [email]; }
        actions {
            read read { primary; }
            update replace_password {
                require_atomic false;
                argument password: String;
                argument password_confirmation: String;
                change custom(&ash_authentication::HashPasswordChange::new("password", Some("password_confirmation"), "hashed_password"));
            }
            update set_hash { accept [hashed_password, credential_version]; }
            update set_key { accept [api_key_hash]; }
            destroy destroy { primary; }
        }
    }
}

struct Fixture<D: DataLayer> {
    data: D,
    runtime: Runtime<D>,
    ctx: Context<D>,
    strategy: AuthStrategy<User>,
    user: User,
}

async fn fixture<D: DataLayer + Clone>(data: D) -> Fixture<D> {
    let runtime = Runtime::new(data.clone());
    let ctx = runtime.bind(None, Some("one".into()));
    let strategy = User::auth_strategy(&runtime).with_jwt_service(JwtService::new(SECRET));
    let user = User::register_with_password(&ctx)
        .email(EMAIL)
        .password(PASSWORD)
        .password_confirmation(PASSWORD)
        .await
        .unwrap();
    Fixture {
        data,
        runtime,
        ctx,
        strategy,
        user,
    }
}

macro_rules! on_backends {
    ($scenario:ident) => {
        mod $scenario {
            use ash_core::Resource;
            #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
            async fn memory() {
                super::$scenario(super::fixture(super::Memory::new()).await).await;
            }
            #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
            async fn sqlite() {
                let data = super::Sqlite::memory().await.unwrap();
                data.install(&[&super::User::DEF, &super::AshToken::DEF])
                    .await
                    .unwrap();
                super::$scenario(super::fixture(data).await).await;
            }
            #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
            async fn postgres() {
                let url = std::env::var("DATABASE_URL").expect("DATABASE_URL is required");
                let admin = ash_postgres::Postgres::connect(&url).await.unwrap();
                let schema = format!("sessions_{}", uuid::Uuid::new_v4().simple());
                admin
                    .execute_sql(&format!("CREATE SCHEMA {schema}"))
                    .await
                    .unwrap();
                let separator = if url.contains('?') { '&' } else { '?' };
                let data = ash_postgres::Postgres::connect(&format!(
                    "{url}{separator}options=-c%20search_path%3D{schema}%2Cpublic"
                ))
                .await
                .unwrap();
                data.install(&[&super::User::DEF, &super::AshToken::DEF])
                    .await
                    .unwrap();
                super::$scenario(super::fixture(data).await).await;
                admin
                    .execute_sql(&format!("DROP SCHEMA {schema} CASCADE"))
                    .await
                    .unwrap();
            }
        }
    };
}

async fn session<D: DataLayer>(f: &Fixture<D>, password: &str) -> ash_authentication::TokenPair {
    let (_, pair) = f
        .strategy
        .sign_in_with_password_and_token_pair(&f.ctx, EMAIL, password)
        .await
        .unwrap();
    assert!(
        f.strategy
            .authenticate(&f.ctx, &pair.access_token)
            .await
            .is_ok()
    );
    pair
}

async fn assert_invalidated<D: DataLayer>(f: &Fixture<D>, pair: &ash_authentication::TokenPair) {
    assert!(
        f.strategy
            .authenticate(&f.ctx, &pair.access_token)
            .await
            .is_err()
    );
    assert!(
        f.strategy
            .rotate_refresh_token(&f.ctx, &pair.refresh_token)
            .await
            .is_err()
    );
    assert!(
        f.strategy
            .sign_in_with_password(&f.ctx, EMAIL, PASSWORD)
            .await
            .is_err()
    );
    let pair = session(f, NEW_PASSWORD).await;
    let (_, next) = f
        .strategy
        .rotate_refresh_token(&f.ctx, &pair.refresh_token)
        .await
        .unwrap();
    assert!(
        f.strategy
            .authenticate(&f.ctx, &next.access_token)
            .await
            .is_ok()
    );
}

async fn password_change_invalidates_existing_sessions<D: DataLayer>(f: Fixture<D>) {
    let pair = session(&f, PASSWORD).await;
    f.strategy
        .change_password(&f.ctx, f.user.id, PASSWORD, NEW_PASSWORD, NEW_PASSWORD)
        .await
        .unwrap();
    assert_invalidated(&f, &pair).await;
}
on_backends!(password_change_invalidates_existing_sessions);

async fn password_reset_invalidates_existing_sessions<D: DataLayer>(f: Fixture<D>) {
    let pair = session(&f, PASSWORD).await;
    let (_, token) = f
        .strategy
        .request_password_reset(&f.ctx, EMAIL)
        .await
        .unwrap();
    f.strategy
        .reset_password_with_token(&f.ctx, &token, NEW_PASSWORD, NEW_PASSWORD)
        .await
        .unwrap();
    assert_invalidated(&f, &pair).await;
}
on_backends!(password_reset_invalidates_existing_sessions);

async fn application_password_change_invalidates_existing_sessions<D: DataLayer>(f: Fixture<D>) {
    let pair = session(&f, PASSWORD).await;
    f.user
        .replace_password_on(&f.ctx)
        .password(NEW_PASSWORD)
        .password_confirmation(NEW_PASSWORD)
        .await
        .unwrap();
    assert_invalidated(&f, &pair).await;
}
on_backends!(application_password_change_invalidates_existing_sessions);

async fn bulk_password_change_invalidates_existing_sessions<D: TransactionSupport + 'static>(
    f: Fixture<D>,
) {
    let pair = session(&f, PASSWORD).await;
    let fields = FieldMap::from([
        ("password".into(), NEW_PASSWORD.into()),
        ("password_confirmation".into(), NEW_PASSWORD.into()),
    ]);
    let result = ash_core::bulk_update(
        &f.ctx,
        "replace_password",
        [(f.user.clone(), fields)],
        ash_core::BulkUpdateOptions::new(),
    )
    .await
    .unwrap();
    assert_eq!(result.count, 1, "{:?}", result.errors);
    assert_invalidated(&f, &pair).await;
}
on_backends!(bulk_password_change_invalidates_existing_sessions);

async fn hash_writes_version_only_when_the_stored_hash_changes<D: DataLayer>(f: Fixture<D>) {
    let pair = session(&f, PASSWORD).await;
    let original = f.strategy.get_user_by_id(&f.ctx, f.user.id).await.unwrap();
    assert_eq!(original.credential_version, 0);
    f.user
        .set_hash_on(&f.ctx)
        .credential_version(99)
        .await
        .unwrap();
    assert_eq!(
        f.strategy
            .get_user_by_id(&f.ctx, f.user.id)
            .await
            .unwrap()
            .credential_version,
        0
    );
    f.user
        .set_hash_on(&f.ctx)
        .hashed_password(original.hashed_password.clone())
        .credential_version(99)
        .await
        .unwrap();
    assert!(
        f.strategy
            .authenticate(&f.ctx, &pair.access_token)
            .await
            .is_ok()
    );
    let hash = f
        .strategy
        .password_service()
        .hash_password(NEW_PASSWORD)
        .unwrap();
    f.user
        .set_hash_on(&f.ctx)
        .hashed_password(Some(hash))
        .credential_version(0)
        .await
        .unwrap();
    assert_eq!(
        f.strategy
            .get_user_by_id(&f.ctx, f.user.id)
            .await
            .unwrap()
            .credential_version,
        1
    );
    assert_invalidated(&f, &pair).await;
}
on_backends!(hash_writes_version_only_when_the_stored_hash_changes);

async fn password_changes_preserve_independent_api_keys<D: DataLayer>(f: Fixture<D>) {
    let (key, hash) = f.strategy.api_key_service().generate_api_key();
    f.user
        .set_key_on(&f.ctx)
        .api_key_hash(Some(hash))
        .await
        .unwrap();
    assert!(f.strategy.authenticate_api_key(&f.ctx, &key).await.is_ok());
    f.strategy
        .change_password(&f.ctx, f.user.id, PASSWORD, NEW_PASSWORD, NEW_PASSWORD)
        .await
        .unwrap();
    assert!(f.strategy.authenticate_api_key(&f.ctx, &key).await.is_ok());
    let keyed = f.strategy.get_user_by_id(&f.ctx, f.user.id).await.unwrap();
    keyed.set_key_on(&f.ctx).api_key_hash(None).await.unwrap();
    assert!(f.strategy.authenticate_api_key(&f.ctx, &key).await.is_err());
}
on_backends!(password_changes_preserve_independent_api_keys);

async fn revocation_is_shared_by_independent_runtimes<D: DataLayer + Clone>(f: Fixture<D>) {
    let pair = session(&f, PASSWORD).await;
    let valid = session(&f, PASSWORD).await;
    let second = Runtime::new(f.data.clone());
    let ctx = second.bind(None, Some("one".into()));
    let strategy = User::auth_strategy(&second).with_jwt_service(JwtService::new(SECRET));
    assert!(
        strategy
            .authenticate(&ctx, &pair.access_token)
            .await
            .is_ok()
    );
    f.strategy
        .revoke_token(&f.ctx, &pair.access_token)
        .await
        .unwrap();
    assert!(
        strategy
            .authenticate(&ctx, &pair.access_token)
            .await
            .is_err()
    );
    drop(strategy);
    drop(second);
    let restarted = Runtime::new(f.data);
    let strategy = User::auth_strategy(&restarted).with_jwt_service(JwtService::new(SECRET));
    assert!(
        strategy
            .authenticate(
                &restarted.bind(None, Some("one".into())),
                &pair.access_token
            )
            .await
            .is_err()
    );
    assert!(
        strategy
            .authenticate(
                &restarted.bind(None, Some("one".into())),
                &valid.access_token
            )
            .await
            .is_ok()
    );
}
on_backends!(revocation_is_shared_by_independent_runtimes);

async fn refresh_rotation_rejects_reuse_and_preserves_valid_sessions<D: DataLayer>(f: Fixture<D>) {
    let pair = session(&f, PASSWORD).await;
    let (_, rotated) = f
        .strategy
        .rotate_refresh_token(&f.ctx, &pair.refresh_token)
        .await
        .unwrap();
    assert!(
        f.strategy
            .authenticate(&f.ctx, &pair.access_token)
            .await
            .is_ok()
    );
    assert!(
        f.strategy
            .authenticate(&f.ctx, &rotated.access_token)
            .await
            .is_ok()
    );
    assert!(
        f.strategy
            .rotate_refresh_token(&f.ctx, &pair.refresh_token)
            .await
            .is_err()
    );
    assert!(
        f.strategy
            .rotate_refresh_token(&f.ctx, &rotated.refresh_token)
            .await
            .is_ok()
    );
}
on_backends!(refresh_rotation_rejects_reuse_and_preserves_valid_sessions);

async fn parallel_reset_consumes_one_token_once<D: DataLayer + 'static>(f: Fixture<D>) {
    let (_, token) = f
        .strategy
        .request_password_reset(&f.ctx, EMAIL)
        .await
        .unwrap();
    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let (strategy, ctx, token, barrier) = (
            f.strategy.clone(),
            f.ctx.clone(),
            token.clone(),
            barrier.clone(),
        );
        tasks.spawn(async move {
            barrier.wait().await;
            strategy
                .reset_password_with_token(&ctx, &token, NEW_PASSWORD, NEW_PASSWORD)
                .await
                .is_ok()
        });
    }
    let mut successes = 0;
    while let Some(result) = tasks.join_next().await {
        successes += usize::from(result.unwrap());
    }
    assert_eq!(successes, 1);
    assert_eq!(
        f.strategy
            .get_user_by_id(&f.ctx, f.user.id)
            .await
            .unwrap()
            .credential_version,
        1
    );
    session(&f, NEW_PASSWORD).await;
}
on_backends!(parallel_reset_consumes_one_token_once);

async fn parallel_refresh_consumes_one_token_once<D: DataLayer + 'static>(f: Fixture<D>) {
    let pair = session(&f, PASSWORD).await;
    let barrier = Arc::new(tokio::sync::Barrier::new(8));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let (strategy, ctx, token, barrier) = (
            f.strategy.clone(),
            f.ctx.clone(),
            pair.refresh_token.clone(),
            barrier.clone(),
        );
        tasks.spawn(async move {
            barrier.wait().await;
            strategy.rotate_refresh_token(&ctx, &token).await
        });
    }
    let mut successes = 0;
    while let Some(result) = tasks.join_next().await {
        if let Ok((_, pair)) = result.unwrap() {
            successes += 1;
            assert!(
                f.strategy
                    .authenticate(&f.ctx, &pair.access_token)
                    .await
                    .is_ok()
            );
        }
    }
    assert_eq!(successes, 1);
    assert!(
        f.strategy
            .rotate_refresh_token(&f.ctx, &pair.refresh_token)
            .await
            .is_err()
    );
}
on_backends!(parallel_refresh_consumes_one_token_once);

async fn deleted_subject_cannot_authenticate<D: DataLayer + 'static>(f: Fixture<D>) {
    let pair = session(&f, PASSWORD).await;
    #[cfg(feature = "axum")]
    let app: axum::Router = ash_authentication::auth_router(
        f.strategy.clone(),
        Arc::new(JwtService::new(SECRET)),
        f.ctx.clone(),
    );
    f.user.destroy_on(&f.ctx).await.unwrap();
    assert!(
        f.strategy
            .authenticate(&f.ctx, &pair.access_token)
            .await
            .is_err()
    );
    #[cfg(feature = "axum")]
    {
        use tower::ServiceExt;
        let request = axum::http::Request::builder()
            .uri("/me")
            .header("authorization", format!("Bearer {}", pair.access_token))
            .body(axum::body::Body::empty())
            .unwrap();
        assert_eq!(app.oneshot(request).await.unwrap().status(), 401);
    }
}
on_backends!(deleted_subject_cannot_authenticate);

async fn token_namespaces_separate_subject_resource_and_optional_tenant<D: DataLayer>(
    f: Fixture<D>,
) {
    let pair = session(&f, PASSWORD).await;
    let jwt = JwtService::new(SECRET);
    let claims = jwt.verify_token(&pair.access_token).unwrap();
    let original = f.strategy.token_store(&f.ctx);
    let other_tenant = f.runtime.bind(None, Some("null".into()));
    let no_tenant = f.runtime.bootstrap();
    for context in [&other_tenant, &no_tenant] {
        let other = Claims {
            tenant: context.tenant().map(str::to_owned),
            ..claims.clone()
        };
        let store = f.strategy.token_store(context);
        store.store_token(&other).await.unwrap();
        store.revoke_token(&other).await.unwrap();
        assert!(original.get_valid_token(&claims).await.unwrap().is_some());
    }
    let other = Claims {
        sub: Uuid::new_v4(),
        ..claims.clone()
    };
    original.store_token(&other).await.unwrap();
    original.revoke_token(&other).await.unwrap();
    assert!(
        f.strategy
            .authenticate(&f.ctx, &pair.access_token)
            .await
            .is_ok()
    );
    let store = ash_authentication::DatabaseTokenStore::new(
        f.ctx.clone(),
        "OtherUser",
        f.runtime
            .credential_writes::<AshToken>(["revoked", "consumed"]),
    );
    let other = Claims {
        resource: "OtherUser".into(),
        ..claims.clone()
    };
    store.store_token(&other).await.unwrap();
    store.revoke_token(&other).await.unwrap();
    assert!(
        f.strategy
            .authenticate(&f.ctx, &pair.access_token)
            .await
            .is_ok()
    );
    assert!(
        original.store_token(&claims).await.is_err(),
        "namespace must be unique"
    );
}
on_backends!(token_namespaces_separate_subject_resource_and_optional_tenant);

async fn invalid_reset_input_does_not_consume_the_token<D: DataLayer>(f: Fixture<D>) {
    let (_, token) = f
        .strategy
        .request_password_reset(&f.ctx, EMAIL)
        .await
        .unwrap();
    assert!(
        f.strategy
            .reset_password_with_token(&f.ctx, &token, "short", "short")
            .await
            .is_err()
    );
    assert!(
        f.strategy
            .reset_password_with_token(&f.ctx, &token, NEW_PASSWORD, "mismatch")
            .await
            .is_err()
    );
    assert!(
        f.strategy
            .reset_password_with_token(&f.ctx, &token, NEW_PASSWORD, NEW_PASSWORD)
            .await
            .is_ok()
    );
}
on_backends!(invalid_reset_input_does_not_consume_the_token);

/// Failure injection uses the same backend transaction, including its rollback.
#[derive(Clone)]
struct FailingStore<D> {
    data: D,
    failure: Arc<std::sync::atomic::AtomicU8>,
}

impl<D: DataLayer + Clone> DataLayer for FailingStore<D> {
    async fn create(&self, plan: &ash_core::CreatePlan<'_>) -> ash_core::Result<FieldMap> {
        let mode = self.failure.load(std::sync::atomic::Ordering::SeqCst);
        if plan.resource().name == AshToken::DEF.name
            && (mode == 4
                || (mode == 2
                    && plan
                        .fields()
                        .get("purpose")
                        .and_then(ash_core::Value::as_str)
                        == Some("refresh")))
        {
            return Err(ash_core::Error::DataLayer(
                "token persistence failed".into(),
            ));
        }
        self.data.create(plan).await
    }
    async fn update(&self, plan: &ash_core::UpdatePlan<'_>) -> ash_core::Result<FieldMap> {
        self.data.update(plan).await
    }
    async fn destroy(&self, plan: &ash_core::DestroyPlan<'_>) -> ash_core::Result<()> {
        self.data.destroy(plan).await
    }
    async fn run_query(&self, plan: &ash_core::QueryPlan<'_>) -> ash_core::Result<Vec<FieldMap>> {
        self.data.run_query(plan).await
    }
    fn can_update_atomically(&self, resource: &ash_core::ResourceDef) -> bool {
        self.data.can_update_atomically(resource)
    }
    async fn update_atomic(
        &self,
        plan: &ash_core::AtomicUpdatePlan<'_>,
    ) -> ash_core::Result<Vec<FieldMap>> {
        let mode = self.failure.load(std::sync::atomic::Ordering::SeqCst);
        if (mode == 1
            && plan.resource().name == AshToken::DEF.name
            && plan.update().set.iter().any(|(name, _)| name == "revoked"))
            || (mode == 3 && plan.resource().name == User::DEF.name)
        {
            return Err(ash_core::Error::DataLayer("credential write failed".into()));
        }
        self.data.update_atomic(plan).await
    }
    async fn in_transaction<T, F, Fut>(&self, work: F) -> ash_core::Result<T>
    where
        F: FnOnce(Option<Self>) -> Fut + Send,
        Fut: Future<Output = ash_core::Result<T>> + Send,
        T: Send,
    {
        let failure = self.failure.clone();
        self.data
            .in_transaction(move |data| work(data.map(|data| Self { data, failure })))
            .await
    }
}

async fn failed_reset_rolls_back_consumption_password_and_revocation<D: DataLayer + Clone>(
    f: Fixture<D>,
) {
    let failure = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let runtime = Runtime::new(FailingStore {
        data: f.data.clone(),
        failure: failure.clone(),
    });
    let ctx = runtime.bind(None, Some("one".into()));
    let strategy = User::auth_strategy(&runtime).with_jwt_service(JwtService::new(SECRET));
    let pair = session(&f, PASSWORD).await;
    let (_, token) = strategy.request_password_reset(&ctx, EMAIL).await.unwrap();
    for mode in [1, 3] {
        failure.store(mode, std::sync::atomic::Ordering::SeqCst);
        assert!(
            strategy
                .reset_password_with_token(&ctx, &token, NEW_PASSWORD, NEW_PASSWORD)
                .await
                .is_err()
        );
        assert!(
            f.strategy
                .authenticate(&f.ctx, &pair.access_token)
                .await
                .is_ok()
        );
        let user = f.strategy.get_user_by_id(&f.ctx, f.user.id).await.unwrap();
        assert_eq!(user.credential_version, 0);
        assert!(
            f.strategy
                .sign_in_with_password(&f.ctx, EMAIL, PASSWORD)
                .await
                .is_ok()
        );
        let claims = strategy
            .jwt_service()
            .unwrap()
            .verify_token(&token)
            .unwrap();
        assert!(
            strategy
                .token_store(&ctx)
                .get_valid_token(&claims)
                .await
                .unwrap()
                .is_some()
        );
    }
    failure.store(0, std::sync::atomic::Ordering::SeqCst);
    strategy
        .reset_password_with_token(&ctx, &token, NEW_PASSWORD, NEW_PASSWORD)
        .await
        .unwrap();
    assert_invalidated(&f, &pair).await;
}
on_backends!(failed_reset_rolls_back_consumption_password_and_revocation);

async fn failed_refresh_successor_rolls_back_consumption<D: DataLayer + Clone>(f: Fixture<D>) {
    let failure = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let runtime = Runtime::new(FailingStore {
        data: f.data.clone(),
        failure: failure.clone(),
    });
    let ctx = runtime.bind(None, Some("one".into()));
    let strategy = User::auth_strategy(&runtime).with_jwt_service(JwtService::new(SECRET));
    let pair = session(&f, PASSWORD).await;
    failure.store(2, std::sync::atomic::Ordering::SeqCst);
    assert!(
        strategy
            .rotate_refresh_token(&ctx, &pair.refresh_token)
            .await
            .is_err()
    );
    let claims = strategy
        .jwt_service()
        .unwrap()
        .verify_token(&pair.refresh_token)
        .unwrap();
    assert!(
        strategy
            .token_store(&ctx)
            .get_valid_token(&claims)
            .await
            .unwrap()
            .is_some()
    );
    failure.store(0, std::sync::atomic::Ordering::SeqCst);
    assert!(
        strategy
            .rotate_refresh_token(&ctx, &pair.refresh_token)
            .await
            .is_ok()
    );
}
on_backends!(failed_refresh_successor_rolls_back_consumption);

#[tokio::test]
async fn token_and_subject_stores_require_one_transactional_backend() {
    let data = ash_core::StoreRegistry::new().with_default(Memory::new());
    let f = fixture(data).await;
    let result = f
        .strategy
        .sign_in_with_password_and_token_pair(&f.ctx, EMAIL, PASSWORD)
        .await;
    assert!(matches!(
        result,
        Err(ash_authentication::AuthError::Core(
            ash_core::Error::AtomicityUnsupported
        ))
    ));
    assert_eq!(AshToken::query(&f.ctx).all().await.unwrap().len(), 0);
}

#[tokio::test]
async fn pre_version_tokens_are_rejected() {
    use serde_json::json;
    let jwt = JwtService::new(SECRET);
    let claims = json!({ "sub": Uuid::new_v4(), "exp": 4102444800_u64, "iat": 0, "jti": "legacy", "purpose": "access" });
    // A signed JWT without the mandatory resource/version claims cannot be upgraded
    // by trusting its identity or by creating a token row.
    let key = jsonwebtoken::EncodingKey::from_secret(SECRET.as_bytes());
    let token = jsonwebtoken::encode(&jsonwebtoken::Header::default(), &claims, &key).unwrap();
    assert!(jwt.verify_token(&token).is_err());
}

#[tokio::test]
async fn reset_mail_observes_committed_token_and_is_not_sent_after_failed_persistence() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let data = Memory::new();
    let failure = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let runtime = Runtime::new(FailingStore {
        data: data.clone(),
        failure: failure.clone(),
    });
    let ctx = runtime.bind(None, Some("one".into()));
    User::register_with_password(&ctx)
        .email(EMAIL)
        .password(PASSWORD)
        .password_confirmation(PASSWORD)
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let reader = Runtime::new(data);
    let read_ctx = reader.bind(None, Some("one".into()));
    let read_strategy = User::auth_strategy(&reader).with_jwt_service(JwtService::new(SECRET));
    let sender = ash_authentication::CallbackSender::new({
        let calls = calls.clone();
        move |_purpose: &str,
              _user: &FieldMap,
              token: &str|
              -> ash_authentication::BoxFuture<'static, ash_authentication::Result<()>> {
            let token = token.to_owned();
            let (calls, read_ctx, read_strategy) =
                (calls.clone(), read_ctx.clone(), read_strategy.clone());
            Box::pin(async move {
                let claims = read_strategy.jwt_service().unwrap().verify_token(&token)?;
                assert!(
                    read_strategy
                        .token_store(&read_ctx)
                        .get_valid_token(&claims)
                        .await?
                        .is_some()
                );
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }
    });
    let strategy = User::auth_strategy(&runtime)
        .with_jwt_service(JwtService::new(SECRET))
        .with_sender(sender);
    failure.store(4, Ordering::SeqCst);
    assert!(strategy.request_password_reset(&ctx, EMAIL).await.is_err());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    failure.store(0, Ordering::SeqCst);
    assert!(strategy.request_password_reset(&ctx, EMAIL).await.is_ok());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        ctx.required_transaction(|tx| async move {
            strategy
                .request_password_reset(&tx, EMAIL)
                .await
                .map_err(ash_core::Error::from)
        })
        .await
        .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

async fn conditional_consume_checks_purpose_subject_tenant_and_expiry<D: DataLayer>(f: Fixture<D>) {
    let (_, token) = f
        .strategy
        .request_password_reset(&f.ctx, EMAIL)
        .await
        .unwrap();
    let claims = f
        .strategy
        .jwt_service()
        .unwrap()
        .verify_token(&token)
        .unwrap();
    let store = f.strategy.token_store(&f.ctx);
    for wrong in [
        Claims {
            purpose: "refresh".into(),
            ..claims.clone()
        },
        Claims {
            sub: Uuid::new_v4(),
            ..claims.clone()
        },
        Claims {
            tenant: None,
            ..claims.clone()
        },
        Claims {
            resource: "OtherUser".into(),
            ..claims.clone()
        },
    ] {
        let strategy = &f.strategy;
        assert!(
            f.ctx
                .required_transaction(|tx| async move {
                    strategy
                        .token_store(&tx)
                        .consume(&wrong)
                        .await
                        .map_err(ash_core::Error::from)
                })
                .await
                .is_err()
        );
        assert!(store.get_valid_token(&claims).await.unwrap().is_some());
    }
    let expired = Claims {
        exp: 1,
        jti: Uuid::new_v4().to_string(),
        ..claims
    };
    store.store_token(&expired).await.unwrap();
    assert!(store.consume(&expired).await.is_err());
    assert!(
        f.strategy
            .reset_password_with_token(&f.ctx, &token, NEW_PASSWORD, NEW_PASSWORD)
            .await
            .is_ok()
    );
}
on_backends!(conditional_consume_checks_purpose_subject_tenant_and_expiry);

async fn upsert_password_conflicts_increment_the_stored_version<D: TransactionSupport + 'static>(
    f: Fixture<D>,
) {
    for (index, (bulk, password, next)) in [
        (false, PASSWORD, NEW_PASSWORD),
        (true, NEW_PASSWORD, "third-password-789"),
    ]
    .into_iter()
    .enumerate()
    {
        let pair = session(&f, password).await;
        if bulk {
            let input = FieldMap::from([
                ("email".into(), EMAIL.into()),
                ("password".into(), next.into()),
                ("password_confirmation".into(), next.into()),
            ]);
            let result = ash_core::bulk_create::<User, _, _, _>(
                &f.ctx,
                "register_with_password",
                [input],
                ash_core::BulkCreateOptions::new().upsert("unique_email", &["hashed_password"]),
            )
            .await
            .unwrap();
            assert_eq!(result.count, 1, "{:?}", result.errors);
        } else {
            let user = User::register_with_password(&f.ctx)
                .email(EMAIL)
                .password(next)
                .password_confirmation(next)
                .upsert_on("unique_email", &["hashed_password"])
                .await
                .unwrap();
            assert_eq!(user.id, f.user.id);
        }
        assert_eq!(
            f.strategy
                .get_user_by_id(&f.ctx, f.user.id)
                .await
                .unwrap()
                .credential_version,
            index as i64 + 1
        );
        assert!(
            f.strategy
                .authenticate(&f.ctx, &pair.access_token)
                .await
                .is_err()
        );
        session(&f, next).await;
    }
}
on_backends!(upsert_password_conflicts_increment_the_stored_version);

async fn raw_jti_revocation_remains_available_without_a_signer<D: DataLayer>(f: Fixture<D>) {
    let pair = session(&f, PASSWORD).await;
    let claims = f
        .strategy
        .jwt_service()
        .unwrap()
        .verify_token(&pair.access_token)
        .unwrap();
    let strategy = User::auth_strategy(&f.runtime);
    strategy.revoke_token(&f.ctx, &claims.jti).await.unwrap();
    assert!(
        f.strategy
            .authenticate(&f.ctx, &pair.access_token)
            .await
            .is_err()
    );
    session(&f, PASSWORD).await;
}
on_backends!(raw_jti_revocation_remains_available_without_a_signer);
