use ash_authentication::{JwtService, authentication};
use ash_core::{Context, Result};
use ash_mailer::{
    AuthMailerSender, ConsoleMailer, ContextMailerExt, Email, EmailNotifier, Mailer, MailerError,
    MemoryMailer,
};
use ash_memory::Memory;
use uuid::Uuid;

// Define an Order resource in its own submodule
mod order_mod {
    use super::*;

    ash_core::resource! {
        Order {
        table "orders";

        attributes {
            id: Uuid [pk];
            customer_email: String;
            amount: i64;
            status: String = "pending";
        }

        actions {
            create place {
                primary;
                accept [customer_email, amount];
            }

            read read {
                primary;
            }

            update complete {
                primary;
                change set(status, "completed");
            }
        }
    }}
}
pub use order_mod::Order;

// Define a User resource with #[authentication] in its own submodule
mod user_mod {
    use super::*;

    #[authentication]
    ash_core::resource! {
        User {
        table "users";

        attributes {
            id: Uuid [pk];
            email: String;
        }

        authentication {
            strategy password {
                identity_field: email;
                hashed_password_field: hashed_password;
                min_password_length: 8;
                require_confirmation: true;
            }

            strategy tokens {
                token_lifetime_secs: 3600;
            }
        }

        actions {
            read read { primary; }
        }
    }}
}
pub use user_mod::User;

#[tokio::test]
async fn test_email_builder_and_validation() {
    // 1. Missing sender
    let err = Email::new()
        .to("recipient@example.com")
        .subject("Hello")
        .validate();
    assert!(matches!(err, Err(MailerError::MissingSender)));

    // 2. Missing recipient
    let err = Email::new()
        .from("sender@example.com")
        .subject("Hello")
        .validate();
    assert!(matches!(err, Err(MailerError::MissingRecipient)));

    // 3. Missing subject
    let err = Email::new()
        .from("sender@example.com")
        .to("recipient@example.com")
        .validate();
    assert!(matches!(err, Err(MailerError::MissingSubject)));

    // 4. Valid email with fluent builder
    let email = Email::new()
        .from("sender@example.com")
        .to("alice@example.com")
        .cc("boss@example.com")
        .bcc("archive@example.com")
        .reply_to("support@example.com")
        .subject("Order #1001 Shipped")
        .text("Your order has shipped.")
        .html("<p>Your order has shipped.</p>")
        .header("X-Campaign-ID", "welcome-100");

    assert!(email.validate().is_ok());
    assert_eq!(email.to, vec!["alice@example.com"]);
    assert_eq!(email.cc, vec!["boss@example.com"]);
    assert_eq!(email.headers.get("X-Campaign-ID").unwrap(), "welcome-100");
}

#[tokio::test]
async fn test_memory_mailer_delivery_and_inspection() -> Result<()> {
    let mailer = MemoryMailer::new();

    let email1 = Email::new()
        .from("notifications@store.com")
        .to("alice@example.com")
        .subject("Welcome Alice!")
        .text("Thanks for signing up!");

    let email2 = Email::new()
        .from("notifications@store.com")
        .to("bob@example.com")
        .subject("Welcome Bob!")
        .text("Thanks for signing up!");

    mailer.deliver(email1).await.unwrap();
    mailer.deliver(email2).await.unwrap();

    assert_eq!(mailer.delivered_count(), 2);
    assert!(mailer.has_delivered_to("alice@example.com"));
    assert!(mailer.has_delivered_to("bob@example.com"));
    assert!(!mailer.has_delivered_to("charlie@example.com"));

    let alice_emails = mailer.find_to("alice@example.com");
    assert_eq!(alice_emails.len(), 1);
    assert_eq!(alice_emails[0].subject.as_deref(), Some("Welcome Alice!"));

    let last = mailer.last_delivered().unwrap();
    assert_eq!(last.to, vec!["bob@example.com"]);

    mailer.clear();
    assert_eq!(mailer.delivered_count(), 0);

    Ok(())
}

#[tokio::test]
async fn test_console_mailer_delivery() {
    let console = ConsoleMailer::new().with_tag("[test-suite]");
    let email = Email::new()
        .from("system@app.local")
        .to("admin@app.local")
        .subject("Console Delivery Test")
        .text("Line 1 of body\nLine 2 of body");

    assert!(console.deliver(email).await.is_ok());
}

#[tokio::test]
async fn test_email_notifier_on_resource_action() -> Result<()> {
    let mailer = MemoryMailer::new();

    // Configure EmailNotifier: whenever an Order is "complete", send a confirmation email!
    let notifier = EmailNotifier::new(mailer.clone()).on("complete", |notif| {
        let customer = notif
            .get("customer_email")
            .and_then(|v| v.as_str())
            .unwrap_or("customer@example.com");
        let amount = notif.get("amount").and_then(|v| v.as_int()).unwrap_or(0);
        let id = notif.id;

        Email::new()
            .from("orders@mystore.com")
            .to(customer)
            .subject(format!("Receipt for Order {}", id))
            .text(format!(
                "Thank you for your order! Total amount: ${amount}."
            ))
            .html(format!(
                "<h1>Order Receipt</h1><p>Total amount: <strong>${amount}</strong></p>"
            ))
    });

    let ctx = Context::new(Memory::new()).with_email_notifier(notifier);

    // 1. Create order: "place" action -> should NOT trigger email
    let order = Order::place(&ctx)
        .customer_email("clara@example.com")
        .amount(79)
        .await
        .expect("order placement should succeed");

    assert_eq!(mailer.delivered_count(), 0);

    // 2. Complete order: "complete" action -> triggers EmailNotifier!
    let _completed = order
        .complete_on(&ctx)
        .await
        .expect("order completion should succeed");

    assert_eq!(mailer.delivered_count(), 1);
    assert!(mailer.has_delivered_to("clara@example.com"));

    let email = mailer.last_delivered().unwrap();
    assert_eq!(email.from.as_deref(), Some("orders@mystore.com"));
    let expected_subject = format!("Receipt for Order {}", order.id);
    assert_eq!(email.subject.as_deref(), Some(expected_subject.as_str()));
    assert!(email.text.as_ref().unwrap().contains("$79"));
    assert!(email.html.as_ref().unwrap().contains("<strong>$79</strong>"));

    Ok(())
}

#[tokio::test]
async fn test_auth_mailer_sender_password_reset() -> Result<()> {
    let ctx = Context::new(Memory::new());
    let mailer = MemoryMailer::new();
    let jwt_service = JwtService::new("test-super-secret-auth-mailer-signing-key-123456");

    // 1. Register a user
    let user: User = User::register_with_password(&ctx)
        .email("david@example.com")
        .password("david-password-123")
        .password_confirmation("david-password-123")
        .await
        .expect("user registration should succeed");

    // 2. Configure AuthStrategy with AuthMailerSender
    let auth_sender = AuthMailerSender::new(mailer.clone(), "accounts@myservice.com")
        .with_base_url("https://myservice.com");

    let strategy = User::auth_strategy()
        .with_jwt_service(jwt_service)
        .with_sender(auth_sender);

    // 3. Request password reset
    let (_u, reset_token) = strategy
        .request_password_reset(&ctx, "david@example.com")
        .await
        .expect("password reset request should succeed");

    // 4. Verify email was delivered by AuthMailerSender through MemoryMailer
    assert_eq!(mailer.delivered_count(), 1);
    assert!(mailer.has_delivered_to("david@example.com"));

    let email = mailer.last_delivered().unwrap();
    assert_eq!(email.from.as_deref(), Some("accounts@myservice.com"));
    assert_eq!(email.subject.as_deref(), Some("Password Reset Instructions"));
    assert!(email.text.as_ref().unwrap().contains(&reset_token));
    assert!(
        email
            .text
            .as_ref()
            .unwrap()
            .contains("https://myservice.com/auth/reset-password?token=")
    );
    assert!(
        email
            .html
            .as_ref()
            .unwrap()
            .contains("<a href=\"https://myservice.com/auth/reset-password?token=")
    );

    // 5. Complete password reset with the delivered token
    strategy
        .reset_password_with_token(
            &ctx,
            &reset_token,
            "new-david-password-456",
            "new-david-password-456",
        )
        .await
        .expect("password reset should succeed");

    // 6. Sign in with new password
    let auth = strategy
        .sign_in_with_password(&ctx, "david@example.com", "new-david-password-456")
        .await
        .expect("sign-in with new password should succeed");
    assert_eq!(auth.id, user.id);

    Ok(())
}

#[tokio::test]
async fn test_custom_auth_mailer_template() -> Result<()> {
    let ctx = Context::new(Memory::new());
    let mailer = MemoryMailer::new();
    let jwt_service = JwtService::new("test-super-secret-auth-mailer-signing-key-custom");

    // Register user
    let _user: User = User::register_with_password(&ctx)
        .email("elena@example.com")
        .password("elena-password-123")
        .password_confirmation("elena-password-123")
        .await
        .expect("user registration should succeed");

    // Custom template
    let auth_sender = AuthMailerSender::new(mailer.clone(), "security@customapp.org")
        .with_password_reset_template(|_user_fields, token, _base_url| {
            Email::new()
                .from("security@customapp.org")
                .to("elena@example.com")
                .subject("CUSTOM: Reset your Security Key")
                .text(format!("Custom reset code: {token}"))
        });

    let strategy = User::auth_strategy()
        .with_jwt_service(jwt_service)
        .with_sender(auth_sender);

    let (_u, reset_token) = strategy
        .request_password_reset(&ctx, "elena@example.com")
        .await
        .expect("password reset request should succeed");

    assert_eq!(mailer.delivered_count(), 1);
    let email = mailer.last_delivered().unwrap();
    assert_eq!(
        email.subject.as_deref(),
        Some("CUSTOM: Reset your Security Key")
    );
    let expected_body = format!("Custom reset code: {reset_token}");
    assert_eq!(
        email.text.as_deref(),
        Some(expected_body.as_str())
    );

    Ok(())
}
