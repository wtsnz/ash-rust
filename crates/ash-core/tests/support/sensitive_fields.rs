use ash_core::{ResourceExtension, resource};
use uuid::Uuid;

#[derive(Debug)]
struct CredentialFields;
impl ResourceExtension for CredentialFields {
    fn name(&self) -> &'static str {
        "credential_fields"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn sensitive_attribute(&self, field: &str) -> bool {
        ["password_digest", "api_digest"].contains(&field)
    }
}
static CREDENTIAL_FIELDS: CredentialFields = CredentialFields;
resource! {
    Member {
        table "sensitive_members";
        attributes { id: Uuid [pk]; label: String; password_digest: Option<String>; api_digest: Option<String>; }
        extensions [&CREDENTIAL_FIELDS]
        actions {
            read read { primary; }
            create create { primary; accept [label, password_digest, api_digest]; }
        }
    }
}
