#[path = "../../ash-core/tests/support/sensitive_fields.rs"]
mod sensitive;
use ash_core::Resource;
use sensitive::Member;

#[tokio::test]
async fn sensitive_storage_fields_are_absent_from_graphql_output_inputs_filters_and_sorts() {
    let schema = ash_graphql::AshGraphQL::from_resources(&[&Member::DEF])
        .finish::<ash_memory::Memory>()
        .unwrap();
    let sdl = schema.sdl();
    for name in [
        "passwordDigest",
        "apiDigest",
        "PASSWORD_DIGEST",
        "API_DIGEST",
    ] {
        assert!(!sdl.contains(name), "{sdl}");
    }
    assert!(sdl.contains("label"));
    let response = schema.execute("{ listMembers { passwordDigest } }").await;
    assert!(!response.errors.is_empty());
}
