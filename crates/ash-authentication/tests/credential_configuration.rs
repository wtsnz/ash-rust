use ash_authentication::{AuthStrategy, JwtService, PasswordService};
use ash_core::Runtime;
use ash_memory::Memory;
use uuid::Uuid;

ash_core::resource! {
    BareUser {
        attributes {
            id: Uuid [pk];
            email: String;
            hashed_password: Option<String>;
            credential_version: i64 = 0;
        }
        actions {
            create create { primary; accept [email, hashed_password]; }
            read read { primary; }
        }
    }
}

#[tokio::test]
async fn password_authentication_requires_the_resource_credential_version_rule() {
    let runtime = Runtime::new(Memory::new());
    let context = runtime.bootstrap();
    let passwords = PasswordService::new();
    let hash = passwords.hash_password("member-password-123").unwrap();
    assert!(
        passwords
            .verify_password("member-password-123", &hash)
            .unwrap()
    );
    let user = BareUser::create(&context)
        .email("member@example.test")
        .hashed_password(Some(hash))
        .await
        .unwrap();
    let strategy = AuthStrategy::<BareUser>::new(&runtime)
        .with_jwt_service(JwtService::new("configuration-test-signing-key-0123456789"));
    let stored = strategy.get_user_by_id(&context, user.id).await.unwrap();
    assert_eq!(stored.credential_version, 0);
    for result in [
        strategy
            .sign_in_with_password(&context, "member@example.test", "member-password-123")
            .await,
        strategy
            .change_password(
                &context,
                user.id,
                "member-password-123",
                "replacement-password-456",
                "replacement-password-456",
            )
            .await,
        strategy
            .request_password_reset(&context, "member@example.test")
            .await
            .map(|(user, _)| user),
    ] {
        match result {
            Err(ash_authentication::AuthError::Core(ash_core::Error::Invalid(message))) => {
                assert!(message.contains("credential version rule"))
            }
            other => panic!("missing authentication write rule was accepted: {other:?}"),
        }
    }
    assert!(
        strategy
            .sign_in_with_password_and_token_pair(
                &context,
                "member@example.test",
                "member-password-123"
            )
            .await
            .is_err()
    );
    assert!(
        ash_authentication::AshToken::query(&context)
            .all()
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(BareUser::query(&context).all().await.unwrap().len(), 1);
}
