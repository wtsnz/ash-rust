#[path = "../../ash-core/tests/support/sensitive_fields.rs"]
mod sensitive;
use ash_core::Resource;
use sensitive::Member;

#[test]
fn sensitive_storage_fields_are_absent_from_typescript_interfaces_and_zod_schemas() {
    let interface = ash_typescript::types::generate_resource_interface(&Member::DEF);
    let schema = ash_typescript::zod::generate_resource_zod_schema(&Member::DEF);
    let input =
        ash_typescript::types::generate_action_input_interface(&Member::DEF, "create").unwrap();
    let input_schema = ash_typescript::zod::generate_action_zod_schema(
        &Member::DEF,
        Member::DEF.action("create").unwrap(),
    )
    .unwrap();
    let filter = ash_typescript::types::generate_resource_filter_input(&Member::DEF);
    let sort = ash_typescript::types::generate_resource_sort_input(&Member::DEF);
    for output in [interface, schema, input, input_schema, filter, sort] {
        assert!(output.contains("label"));
        for name in [
            "passwordDigest",
            "apiDigest",
            "password_digest",
            "api_digest",
        ] {
            assert!(!output.contains(name), "{output}");
        }
    }
}

#[tokio::test]
async fn generated_client_selections_query_the_public_graphql_schema() {
    let builder = ash_typescript::client::generate_selection_set_builder(&Member::DEF);
    let selection = builder
        .lines()
        .find_map(|line| line.trim().strip_prefix("let fields = \""))
        .and_then(|rest| rest.strip_suffix("\";"))
        .unwrap();
    let context = ash_core::Runtime::new(ash_memory::Memory::new()).bootstrap();
    Member::create(&context)
        .label("member")
        .password_digest(Some("stored-password-hash".into()))
        .api_digest(Some("stored-api-key-hash".into()))
        .await
        .unwrap();
    let schema = ash_graphql::AshGraphQL::from_resources(&[&Member::DEF])
        .finish::<ash_memory::Memory>()
        .unwrap();
    let response = schema
        .execute(
            async_graphql::Request::new(format!("{{ listMembers {{ {selection} }} }}"))
                .data(context),
        )
        .await;
    assert!(
        response.errors.is_empty(),
        "{selection}: {:?}",
        response.errors
    );
    let data = response.data.into_json().unwrap();
    assert_eq!(data["listMembers"][0]["label"], "member");
    for name in ["passwordDigest", "apiDigest"] {
        assert!(
            !data["listMembers"][0]
                .as_object()
                .unwrap()
                .contains_key(name)
        );
    }
}
