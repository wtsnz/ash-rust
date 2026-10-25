use ash_authentication::{AshToken, AuthStrategy, DatabaseTokenStore, JwtService, authentication};
use ash_core::{DataLayer, FieldMap, Resource, ResourceDef, Runtime};
use ash_memory::Memory;
use uuid::Uuid;

const SECRET: &str = "store-consistency-test-signing-key-0123456789";

#[authentication]
ash_core::resource! {
    User {
        attributes { id: Uuid [pk]; email: String; }
        authentication { strategy password { identity_field: email; } strategy tokens {} }
        actions { read read { primary; } }
    }
}

/// Resource routing without a transaction/consistency protocol.
struct SeparateStores {
    subjects: Memory,
    tokens: Memory,
}
impl SeparateStores {
    fn store(&self, resource: &ResourceDef) -> &Memory {
        if resource.name == AshToken::DEF.name {
            &self.tokens
        } else {
            &self.subjects
        }
    }
}
impl DataLayer for SeparateStores {
    async fn create(&self, plan: &ash_core::CreatePlan<'_>) -> ash_core::Result<FieldMap> {
        self.store(plan.resource()).create(plan).await
    }
    async fn update(&self, plan: &ash_core::UpdatePlan<'_>) -> ash_core::Result<FieldMap> {
        self.store(plan.resource()).update(plan).await
    }
    async fn destroy(&self, plan: &ash_core::DestroyPlan<'_>) -> ash_core::Result<()> {
        self.store(plan.resource()).destroy(plan).await
    }
    async fn run_query(&self, plan: &ash_core::QueryPlan<'_>) -> ash_core::Result<Vec<FieldMap>> {
        self.store(plan.resource()).run_query(plan).await
    }
}

#[tokio::test]
async fn jwt_authentication_rejects_subject_and_token_stores_without_a_consistency_protocol() {
    let subjects = Memory::new();
    let producer = Runtime::new(subjects.clone());
    let context = producer.bootstrap();
    User::register_with_password(&context)
        .email("member@example.test")
        .password("member-password-123")
        .password_confirmation("member-password-123")
        .await
        .unwrap();
    let strategy = AuthStrategy::<User>::new(&producer).with_jwt_service(JwtService::new(SECRET));
    let (_, pair) = strategy
        .sign_in_with_password_and_token_pair(
            &context,
            "member@example.test",
            "member-password-123",
        )
        .await
        .unwrap();
    assert!(
        strategy
            .authenticate(&context, &pair.access_token)
            .await
            .is_ok()
    );

    let tokens = Memory::new();
    let token_runtime = Runtime::new(tokens.clone());
    let store = DatabaseTokenStore::new(
        token_runtime.bootstrap(),
        User::DEF.name,
        token_runtime.credential_writes::<AshToken>(["consumed", "revoked"]),
    );
    let claims = JwtService::new(SECRET)
        .verify_token(&pair.access_token)
        .unwrap();
    store.store_token(&claims).await.unwrap();
    assert!(store.get_valid_token(&claims).await.unwrap().is_some());

    let runtime = Runtime::new(SeparateStores { subjects, tokens });
    let strategy = AuthStrategy::<User>::new(&runtime).with_jwt_service(JwtService::new(SECRET));
    let result = strategy
        .authenticate(&runtime.bootstrap(), &pair.access_token)
        .await;
    assert!(
        matches!(
            result,
            Err(ash_authentication::AuthError::Core(
                ash_core::Error::AtomicityUnsupported
            ))
        ),
        "{result:?}"
    );
}
