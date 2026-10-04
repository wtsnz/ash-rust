What `mix ash_typescript.codegen` writes for the Elixir desk (`examples/elixir/supportdesk`)
with every client option on, for `tests/client_codegen.rs` to compare ash-rust's generator
against. To regenerate: add these to its `config :ash_typescript` (with `output_file`
pointing here), and these typed queries to its `Supportdesk.Desk` Ticket block, run
`mix ash_typescript.codegen`, then revert both.

```elixir
generate_validation_functions: true,
generate_phx_channel_rpc_actions: true,
rpc_action_before_request_hook: "RpcHooks.beforeRequest",
rpc_action_after_request_hook: "RpcHooks.afterRequest",
rpc_validation_before_request_hook: "RpcHooks.beforeValidationRequest",
rpc_validation_after_request_hook: "RpcHooks.afterValidationRequest",
rpc_action_hook_context_type: "RpcHooks.ActionHookContext",
rpc_validation_hook_context_type: "RpcHooks.ValidationHookContext",
rpc_action_before_channel_push_hook: "RpcHooks.beforeChannelPush",
rpc_action_after_channel_response_hook: "RpcHooks.afterChannelResponse",
rpc_validation_before_channel_push_hook: "RpcHooks.beforeValidationChannelPush",
rpc_validation_after_channel_response_hook: "RpcHooks.afterValidationChannelResponse",
rpc_action_channel_hook_context_type: "RpcHooks.ActionChannelHookContext",
rpc_validation_channel_hook_context_type: "RpcHooks.ValidationChannelHookContext",
```

```elixir
typed_query :ticket_inbox, :read do
  fields [:id, :subject, %{assignee: [:name]}]
end

typed_query :ticket_card, :read do
  fields [:id, :status]
end
```
