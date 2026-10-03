# Supportdesk on Ash

The support desk on Elixir Ash, AshPostgres, AshGraphql and AshTypescript: the twin of the
ash-rust desk in [`examples/supportdesk`](../../supportdesk), whose README is the contract
both implement and says how to run them side by side.

`mix ash_typescript.codegen` regenerates the TypeScript RPC client both desks serve,
[`examples/supportdesk/client/ash_rpc.ts`](../../supportdesk/client/ash_rpc.ts).
