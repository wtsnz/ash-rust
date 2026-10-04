//! ash-rust generates the AshTypescript client the Elixir desk does: the same
//! `ash_types.ts` and `ash_rpc.ts`, line for line, from the same actions.

use ash_postgres::Postgres;
use ash_typescript::rpc::ClientConfig;

#[test]
fn the_desk_generates_ash_typescripts_client() {
    let client = supportdesk::server::rpc::<Postgres>().typescript_client(&ClientConfig::default());
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/client");
    for (name, generated) in [("ash_types.ts", &client.types), ("ash_rpc.ts", &client.rpc)] {
        let expected = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
        if *generated != expected {
            let out = std::env::temp_dir().join(format!("rust_{name}"));
            std::fs::write(&out, generated).unwrap();
            panic!("{name} differs from the Elixir desk's: diff {dir}/{name} {}", out.display());
        }
    }
}

/// The client with every option AshTypescript has turned on: validation and channel
/// functions, lifecycle hooks and their context types, and typed queries. The fixture is
/// what `mix ash_typescript.codegen` writes for the Elixir desk with those options set
/// (see `tests/fixtures/client_full/README.md`).
#[test]
fn every_option_generates_as_ash_typescript_does() {
    use ash_typescript::rpc::Hooks;
    let hook = |name: &str| Some(format!("RpcHooks.{name}"));
    let config = ClientConfig {
        validation_functions: true,
        channel_functions: true,
        hooks: Hooks {
            action_before_request: hook("beforeRequest"),
            action_after_request: hook("afterRequest"),
            validation_before_request: hook("beforeValidationRequest"),
            validation_after_request: hook("afterValidationRequest"),
            action_before_channel_push: hook("beforeChannelPush"),
            action_after_channel_response: hook("afterChannelResponse"),
            validation_before_channel_push: hook("beforeValidationChannelPush"),
            validation_after_channel_response: hook("afterValidationChannelResponse"),
            action_context_type: hook("ActionHookContext"),
            validation_context_type: hook("ValidationHookContext"),
            action_channel_context_type: hook("ActionChannelHookContext"),
            validation_channel_context_type: hook("ValidationChannelHookContext"),
        },
        ..ClientConfig::default()
    };
    let rpc = supportdesk::server::rpc::<Postgres>()
        .typed_query::<supportdesk::Ticket>("ticket_inbox", "read", serde_json::json!(["id", "subject", { "assignee": ["name"] }]))
        .typed_query::<supportdesk::Ticket>("ticket_card", "read", serde_json::json!(["id", "status"]));
    let client = rpc.typescript_client(&config);
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/client_full");
    for (name, generated) in [("ash_types.ts", &client.types), ("ash_rpc.ts", &client.rpc)] {
        let expected = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
        if *generated != expected {
            let out = std::env::temp_dir().join(format!("rust_full_{name}"));
            std::fs::write(&out, generated).unwrap();
            panic!("{name} differs from AshTypescript's: diff {dir}/{name} {}", out.display());
        }
    }
}
