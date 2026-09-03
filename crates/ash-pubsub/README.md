# ash-pubsub

[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-orange.svg)](https://crates.io)
[![Documentation](https://docs.rs/ash-pubsub/badge.svg)](https://docs.rs/ash-pubsub)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`ash-pubsub` is a pattern-based, asynchronous PubSub event broker and action notifier for **ash-rust**, modeled directly after Elixir's [`ash_pubsub`](https://github.com/ash-project/ash_pubsub) extension.

It decouples side-effects and event distribution from resource business logic, enabling live updates, WebSocket pushes, background jobs, and audit logs to react to resource lifecycle events with zero coupling.

---

## What This Crate Provides

- **`PubSub` Broker**: High-performance, in-memory event distributor backed by `tokio::sync::broadcast` channels with configurable capacity.
- **Topic Pattern Matching & Wildcards**:
  - Global wildcards: `"*"` (receives all events)
  - Namespace wildcards: `"order:*"` (receives all order actions)
  - Segment wildcards: `"order:*:archived"`
  - Exact topics: `"order:create"`
- **Deduplicated Multi-Topic Dispatch**: When a single mutation matches multiple topic patterns, `publish_topics` guarantees each subscriber receives the notification exactly once.
- **`PubSubNotifier`**: Implements `ash_core::Notifier` to listen to resource actions and route them into `PubSub`.
- **Fluent Ergonomics**:
  - `ContextPubSubExt`: Seamlessly attach PubSub to any execution context via `ctx.with_pubsub(pubsub)`.
  - `PubSubResourceExt`: Typed subscription helpers eliminating string formatting errors:
    - `Order::subscribe_all(&pubsub)` (`"order:*"`)
    - `Order::subscribe_action(&pubsub, "cancel")` (`"order:cancel"`)
    - `order.subscribe(&pubsub, None)` (`"order:<id>:*"`)
- **Transactional Atomicity with `Multi`**:
  - Notifications emitted inside `Multi` transactions are buffered in memory.
  - If a transaction rolls back, all queued events are immediately discarded.
  - If the transaction commits, all events are dispatched in order.

---

## Quick Example

```rust
use std::sync::Arc;
use ash_core::{Context, resource};
use ash_memory::Memory;
use ash_pubsub::{ContextPubSubExt, PubSub, PubSubResourceExt};
use uuid::Uuid;

resource! {
    resource Order;
    table "orders";

    attributes {
        id: Uuid [pk],
        amount: i64,
        status: String,
    }

    actions {
        create create {
            primary;
            accept [amount];
            change set(status = "pending");
        }

        update complete {
            change set(status = "completed");
        }
    }
}

#[tokio::main]
async fn main() -> ash_core::Result<()> {
    // 1. Create a shared PubSub broker
    let pubsub = Arc::new(PubSub::new());

    // 2. Attach PubSub notifier to context
    let ctx = Context::new(Memory::new()).with_pubsub(Arc::clone(&pubsub));

    // 3. Subscribe to typed event streams
    let mut sub_all = Order::subscribe_all(&pubsub);
    let mut sub_creates = Order::subscribe_action(&pubsub, "create");

    // 4. Execute an action
    let order = Order::create(&ctx)
        .amount(150)
        .await?;

    // 5. Receive typed notifications asynchronously
    let notif = sub_creates.recv().await?;
    assert_eq!(notif.action, "create");
    assert_eq!(notif.id, order.id);

    let notif_all = sub_all.recv().await?;
    assert_eq!(notif_all.id, order.id);

    Ok(())
}
```

---

## License

Licensed under the [MIT License](../../LICENSE).
