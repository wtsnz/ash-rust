use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

use ash_core::{
    resource, ChangeContext, Context, CustomChange, CustomValidation, Error,
    Notification, Result, SyncFnNotifier, ValidationContext, Value,
};
use ash_memory::Memory;

pub mod tenant_models {
    use super::*;

    pub struct RequireTenantValidation;
    impl CustomValidation for RequireTenantValidation {
        fn validate(&self, ctx: &ValidationContext<'_>) -> Result<()> {
            if ctx.tenant.is_none() {
                return Err(Error::Validation {
                    field: "tenant".to_string(),
                    message: "tenant context is required".to_string(),
                });
            }
            Ok(())
        }
    }

    pub struct AutoAssignTenantChange;
    impl CustomChange for AutoAssignTenantChange {
        fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()> {
            if let Some(tenant) = ctx.tenant {
                ctx.fields.insert("tenant_id".to_string(), Value::String(tenant.to_string()));
            }
            Ok(())
        }
    }

    resource! {
        resource Document;
        table "documents";

        attributes {
            id: Uuid [pk],
            tenant_id: String,
            title: String,
            body: Option<String>,
        }

        actions {
            create create {
                primary;
                accept [title, body];
                validate custom(&RequireTenantValidation);
                change custom(&AutoAssignTenantChange);
            }

            read read {
                primary;
            }

            update update {
                primary;
                accept [title, body];
            }

            destroy destroy {
                primary;
            }

            action process_document, String {
                argument extra_tag: String;

                run |input| async move {
                    let tenant = input.tenant().unwrap_or("none");
                    let req_id = input
                        .metadata()
                        .get("request_id")
                        .and_then(|v| match v {
                            Value::String(s) => Some(s.as_str()),
                            _ => None,
                        })
                        .unwrap_or("unknown");

                    Ok(format!("tenant:{tenant}|req:{req_id}|tag:{}", input.extra_tag))
                }
            }
        }
    }
}

use tenant_models::Document;

#[tokio::test]
async fn test_context_fluent_tenant_and_metadata() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    assert_eq!(ctx.tenant(), None);
    assert!(ctx.metadata().is_empty());

    let ctx_with_tenant = ctx.with_tenant("org_acme");
    assert_eq!(ctx_with_tenant.tenant(), Some("org_acme"));

    let ctx_cleared = ctx_with_tenant.without_tenant();
    assert_eq!(ctx_cleared.tenant(), None);

    let ctx_with_meta = ctx_with_tenant
        .with_metadata("trace_id", "trace-999")
        .with_metadata("env", "staging");
    assert_eq!(
        ctx_with_meta.get_metadata("trace_id"),
        Some(&Value::String("trace-999".into()))
    );
    assert_eq!(
        ctx_with_meta.get_metadata("env"),
        Some(&Value::String("staging".into()))
    );
}

#[tokio::test]
async fn test_validation_and_change_context_tenant_propagation() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    // 1. Without tenant, custom validation rejects create
    let err = Document::create(&ctx)
        .title("Secret Doc")
        .call()
        .await
        .unwrap_err();

    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "tenant");
            assert_eq!(message, "tenant context is required");
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    // 2. With tenant on context, validation passes and AutoAssignTenantChange sets tenant_id
    let tenant_ctx = ctx.with_tenant("tenant_omega");
    let doc = Document::create(&tenant_ctx)
        .title("Secret Doc")
        .call()
        .await
        .expect("should create successfully");

    assert_eq!(doc.tenant_id, "tenant_omega");
    assert_eq!(doc.title, "Secret Doc");
}

#[tokio::test]
async fn test_action_builder_tenant_override() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    // Call with tenant specified directly on action builder instead of context
    let doc = Document::create(&ctx)
        .tenant("tenant_custom")
        .title("Custom Tenant Doc")
        .call()
        .await
        .expect("should create successfully");

    assert_eq!(doc.tenant_id, "tenant_custom");

    // Override context tenant via builder
    let base_ctx = ctx.with_tenant("tenant_base");
    let doc_override = Document::create(&base_ctx)
        .tenant("tenant_overridden")
        .title("Overridden Doc")
        .call()
        .await
        .expect("should create successfully");

    assert_eq!(doc_override.tenant_id, "tenant_overridden");
}

#[tokio::test]
async fn test_notification_carries_tenant_and_metadata() {
    let mem = Memory::new();
    let notified = Arc::new(AtomicBool::new(false));
    let notified_clone = Arc::clone(&notified);

    let notifier = SyncFnNotifier::new("test_tenant_notifier", move |notif: &Notification| {
        assert_eq!(notif.tenant.as_deref(), Some("tenant_notif"));
        assert_eq!(
            notif.metadata.get("client_ip"),
            Some(&Value::String("127.0.0.1".into()))
        );
        assert_eq!(
            notif.metadata.get("request_id"),
            Some(&Value::String("req_abc".into()))
        );
        notified_clone.store(true, Ordering::SeqCst);
        Ok(())
    });

    let ctx = Context::new(mem)
        .with_tenant("tenant_notif")
        .with_metadata("client_ip", "127.0.0.1")
        .with_metadata("request_id", "req_abc")
        .with_notifier(Arc::new(notifier));

    let doc = Document::create(&ctx)
        .title("Notification Test")
        .call()
        .await
        .unwrap();

    assert_eq!(doc.title, "Notification Test");
    assert!(notified.load(Ordering::SeqCst), "notifier should have received notification with tenant & metadata");
}

#[tokio::test]
async fn test_query_tenant_inheritance_and_overriding() {
    let mem = Memory::new();
    let ctx = Context::new(mem).with_tenant("org_1");

    let q1 = Document::query(&ctx);
    assert_eq!(q1.get_tenant(), Some("org_1"));

    let q2 = q1.clone().tenant("org_2");
    assert_eq!(q2.get_tenant(), Some("org_2"));

    let q3 = q2.without_tenant();
    assert_eq!(q3.get_tenant(), None);
}

#[tokio::test]
async fn test_generic_action_tenant_and_metadata() {
    let mem = Memory::new();
    let ctx = Context::new(mem)
        .with_tenant("corp_alpha")
        .with_metadata("request_id", "req_9999");

    let res = Document::process_document(&ctx)
        .extra_tag("archived")
        .call()
        .await
        .expect("generic action should succeed");

    assert_eq!(res, "tenant:corp_alpha|req:req_9999|tag:archived");
}

#[tokio::test]
async fn test_multi_pipeline_preserves_tenant_and_metadata() {
    let mem = Memory::new();
    let multi_notified = Arc::new(AtomicBool::new(false));
    let notif_flag = Arc::clone(&multi_notified);

    let notifier = SyncFnNotifier::new("multi_tenant_notifier", move |notif: &Notification| {
        if notif.action == "create" {
            assert_eq!(notif.tenant.as_deref(), Some("tenant_multi"));
            assert_eq!(
                notif.metadata.get("batch_id"),
                Some(&Value::String("batch_01".into()))
            );
            notif_flag.store(true, Ordering::SeqCst);
        }
        Ok(())
    });

    let ctx = Context::new(mem)
        .with_tenant("tenant_multi")
        .with_metadata("batch_id", "batch_01")
        .with_notifier(Arc::new(notifier));

    let multi_result = ctx
        .multi()
        .create("doc1", Document::create(&ctx).title("Multi Doc 1"))
        .create("doc2", Document::create(&ctx).title("Multi Doc 2"))
        .commit()
        .await
        .expect("multi commit should succeed");

    let d1: &Document = multi_result.get("doc1").unwrap();
    let d2: &Document = multi_result.get("doc2").unwrap();

    assert_eq!(d1.tenant_id, "tenant_multi");
    assert_eq!(d2.tenant_id, "tenant_multi");
    assert!(multi_notified.load(Ordering::SeqCst));
}
