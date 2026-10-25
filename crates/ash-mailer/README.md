# ash-mailer

[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-orange.svg)](https://crates.io)
[![Documentation](https://docs.rs/ash-mailer/badge.svg)](https://docs.rs/ash-mailer)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`ash-mailer` is a declarative transactional email delivery and action notification engine for **ash-rust**, modeled directly after Elixir's [Swoosh](https://github.com/swoosh/swoosh) and [`AshAuthentication.Sender`](https://hexdocs.pm/ash_authentication/AshAuthentication.Sender.html).

It decouples email delivery, template rendering, and notification side-effects from core resource business logic and authentication flows.

---

## What This Crate Provides

- **`Email` Model & Fluent Builder**: Clean domain object for composing emails with `from`, `to`, `cc`, `bcc`, `reply_to`, `subject`, `text`, `html`, and custom transport headers.
- **Pluggable `Mailer` Trait**:
  - **`MemoryMailer`**: Stores delivered emails in-memory (`Arc<RwLock<Vec<Email>>>`) with convenient test assertion helpers (`delivered_count()`, `find_to("alice@example.com")`, `has_delivered_to(...)`, `clear()`). Matches Phoenix Swoosh's `Swoosh.Adapters.Test`.
  - **`ConsoleMailer`**: Formats and prints outgoing emails with clean ASCII borders to standard output for local development.
  - **`CallbackMailer`**: Closure-based mailer for custom test assertions or bespoke delivery backends.
  - **`NoopMailer`**: Silently drops outgoing emails when delivery is disabled.
- **`EmailNotifier` (`ash_core::Notifier`)**: Declaratively triggers transactional emails when resource actions commit (e.g. `Order::complete` or `Ticket::assign`).
- **`AuthMailerSender` Bridge**: Implements `ash_authentication::AuthSender`, allowing `ash-authentication` to dispatch password resets, account confirmations, and magic links through any configured `Mailer` without coupling authentication to a specific transport.
- **`ContextMailerExt`**: Fluent context helper `ctx.with_email_notifier(notifier)`.

---

## Quick Examples

### 1. Declarative Action Email Notifier

Send an order receipt automatically whenever an `Order::complete` action commits:

```rust
use std::sync::Arc;
use ash_core::{resource};
use ash_mailer::{Email, EmailNotifier, MemoryMailer};
use ash_memory::Memory;

let mailer = MemoryMailer::new();

let notifier = EmailNotifier::new(mailer.clone())
    .on("complete", |notif| {
        let customer = notif.get("customer_email").and_then(|v| v.as_str()).unwrap();
        let amount = notif.get("amount").and_then(|v| v.as_int()).unwrap();

        Email::new()
            .from("orders@mystore.com")
            .to(customer)
            .subject("Your Order Receipt")
            .text(format!("Thank you for your order of ${amount}!"))
            .html(format!("<h1>Receipt</h1><p>Total: <strong>${amount}</strong></p>"))
    });

let runtime = ash_core::Runtime::new(Memory::new());
let grant = runtime.notifier_grant::<Order>(&["customer_email", "amount"])?;
let ctx = runtime.with_notifier(grant.attach(Arc::new(notifier))).bootstrap();

// Executing the action automatically delivers the email:
order.complete_on(&ctx).await?;
assert!(mailer.has_delivered_to("customer@example.com"));
```

---

### 2. Integration with `ash-authentication`

Connect `ash-mailer` to `ash-authentication` via `AuthMailerSender`:

```rust
use ash_authentication::{JwtService, authentication};
use ash_mailer::{AuthMailerSender, ConsoleMailer};

let mailer = ConsoleMailer::new();
let auth_sender = AuthMailerSender::new(mailer, "noreply@myapp.com")
    .with_base_url("https://myapp.com");

let strategy = User::auth_strategy(&runtime)
    .with_jwt_service(jwt_service)
    .with_sender(auth_sender);

// Requesting a password reset automatically delivers the reset email:
let (user, reset_token) = strategy.request_password_reset(&ctx, "user@example.com").await?;
```

---

### 3. Unit Testing with `MemoryMailer`

```rust
let mailer = MemoryMailer::new();
// ... run resource actions or auth flows ...

assert_eq!(mailer.delivered_count(), 1);
assert!(mailer.has_delivered_to("david@example.com"));

let email = mailer.last_delivered().unwrap();
assert_eq!(email.subject.as_deref(), Some("Password Reset Instructions"));
```

## Notification compatibility

Email rules now receive projected Views. An ordinary `with_email_notifier` attachment
reads as anonymous and receives no arguments, actor attributes or automatic metadata.
Use a Runtime-issued `notifier_grant` for fields needed by a privileged mail rule,
as in the receipt example above. A grant applies only to its resource and issuing
Runtime. Password reset/confirmation tokens remain on `AuthMailerSender`'s explicit
sender path. See the [breaking event changes](../../docs/scoped-access.md#events-and-subscriptions).
