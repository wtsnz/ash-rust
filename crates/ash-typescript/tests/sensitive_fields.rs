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
