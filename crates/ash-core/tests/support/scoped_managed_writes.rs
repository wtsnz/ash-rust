use ash_core::{
    Actor, Context, DataLayer, Error, FieldMap, Filter, ManagedRelType, Resource, ResourceExt,
    Runtime, TransactionSupport, Value, fields, resource,
};
use uuid::Uuid;

pub mod document {
    use super::*;
    resource! {
        Document {
            table "scoped_managed_documents";
            multitenancy { strategy: attribute; attribute: org; }
            attributes {
                id: Uuid [pk]; org: String; code: String; owner: Uuid;
                visible: bool [default: true]; title: String;
            }
            relationships {
                has_many remarks: super::remark::Remark [fk: document_id];
                has_many keyed_remarks: super::keyed_remark::KeyedRemark [fk: document_id];
            }
            identities { identity org_code: [org, code]; }
            policies {
                policy action_type(read) { authorize_if relates_to_actor(owner); }
                policy action_type(create) { authorize_if always; }
                policy action_type(update) { authorize_if always; }
            }
            actions {
                create create { primary; accept [code, owner, visible, title]; validate present(code); }
                read read { primary; prepare filter(visible == true); }
                update update { primary; require_atomic false; accept [title]; }
            }
        }
    }
}
pub use document::Document;

pub mod remark {
    use super::*;
    resource! {
        Remark {
            table "scoped_managed_remarks";
            multitenancy { strategy: attribute; attribute: org; }
            attributes { id: Uuid [pk]; org: String; document_id: Option<Uuid>; body: String; }
            relationships { belongs_to document: Document [fk: document_id]; }
            actions {
                create create { primary; accept [document_id, body]; }
                create create_with_document {
                    accept [body]; argument document: FieldMap;
                    change manage_relationship(document, append);
                    after_action fail_write;
                }
                read read { primary; }
                update update { primary; require_atomic false; accept [body]; }
                destroy destroy { primary; }
            }
        }
    }
}
pub use remark::Remark;

pub mod keyed_remark {
    use super::*;
    resource! {
        KeyedRemark {
            table "scoped_managed_keyed_remarks";
            multitenancy { strategy: attribute; attribute: org; }
            attributes { id: String [pk]; org: String; document_id: Uuid; body: String; }
            actions {
                create create { primary; accept [id, body]; }
                read read { primary; }
            }
        }
    }
}
pub use keyed_remark::KeyedRemark;

pub mod composite_remark {
    use super::*;
    resource! {
        CompositeRemark {
            table "scoped_managed_composite_remarks";
            multitenancy { strategy: attribute; attribute: org; }
            attributes {
                id: Uuid [pk]; org: String; document_org: Option<String>;
                document_code: Option<String>; body: String;
            }
            relationships {
                belongs_to document: Document
                    [fk: [document_org, document_code], references: [org, code]];
            }
            actions {
                create create { primary; accept [body]; }
                read read { primary; }
            }
        }
    }
}
pub use composite_remark::CompositeRemark;

pub mod article {
    use super::*;
    resource! {
        Article {
            table "scoped_managed_articles";
            multitenancy { strategy: attribute; attribute: org; }
            attributes { id: Uuid [pk]; org: String; title: String; }
            relationships {
                many_to_many labels: super::label::Label
                    [through: Link, source_fk: article_id, dest_fk: label_id, on_delete: cascade];
                many_to_many global_labels: super::label::Label
                    [through: GlobalLink, source_fk: article_id, dest_fk: label_id];
                many_to_many readonly_labels: super::label::Label
                    [through: ReadonlyLink, source_fk: article_id, dest_fk: label_id];
            }
            actions {
                create create { primary; accept [title]; }
                read read { primary; }
                update update { primary; require_atomic false; accept [title]; }
                update manage_labels {
                    require_atomic false; accept [title]; argument labels: Vec<FieldMap>;
                    change manage_relationship(labels, direct_control);
                }
                destroy destroy { primary; }
            }
        }
    }
}
pub use article::Article;

pub mod label {
    use super::*;
    resource! {
        Label {
            table "scoped_managed_labels";
            multitenancy { strategy: attribute; attribute: org; }
            attributes {
                id: Uuid [pk]; org: String; name: String; owner: Uuid;
                visible: bool [default: true];
            }
            policies {
                policy action_type(read) { authorize_if relates_to_actor(owner); }
                policy action_type(create) { authorize_if always; }
            }
            actions {
                create create { primary; accept [name, owner, visible]; validate present(name); }
                read read { primary; prepare filter(visible == true); }
            }
        }
    }
}
pub use label::Label;

pub mod link {
    use super::*;
    resource! {
        Link {
            table "scoped_managed_links";
            actor { role: String; }
            multitenancy { strategy: attribute; attribute: org; }
            attributes { id: Uuid [pk]; org: Option<String>; article_id: Uuid; label_id: Uuid; }
            policies {
                policy action_type(read) { authorize_if always; }
                policy action_type(create) { authorize_if actor_attribute_equals(role, "linker"); }
                policy action_type(destroy) { authorize_if actor_attribute_equals(role, "linker"); }
            }
            actions {
                create create { primary; accept [article_id, label_id]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}
pub use link::Link;

pub mod global_link {
    use super::*;
    resource! {
        GlobalLink {
            table "scoped_managed_global_links";
            attributes { id: Uuid [pk]; article_id: Uuid; label_id: Uuid; }
            actions {
                create create { primary; accept [article_id, label_id]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}
pub use global_link::GlobalLink;

pub mod readonly_link {
    use super::*;
    resource! {
        ReadonlyLink {
            table "scoped_managed_readonly_links";
            attributes { id: Uuid [pk]; article_id: Uuid; label_id: Uuid; }
            actions { read read { primary; } }
        }
    }
}
pub use readonly_link::ReadonlyLink;

pub mod folder {
    use super::*;
    resource! {
        Folder {
            table "scoped_managed_folders";
            multitenancy { strategy: attribute; attribute: org; }
            attributes { id: Uuid [pk]; org: String; name: String; }
            relationships {
                has_many leaves: super::leaf::Leaf [fk: folder_id, on_delete: cascade];
                has_many tenant_leaves: super::tenant_leaf::TenantLeaf [fk: folder_id, on_delete: cascade];
                has_many blockers: super::blocker::Blocker [fk: folder_id, on_delete: cascade];
            }
            actions {
                create create { primary; accept [name]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}
pub use folder::Folder;

pub mod leaf {
    use super::*;
    resource! {
        Leaf {
            table "scoped_managed_leaves";
            attributes { id: Uuid [pk]; folder_id: Uuid; name: String; }
            actions {
                create create { primary; accept [folder_id, name]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}
pub use leaf::Leaf;

pub mod tenant_leaf {
    use super::*;
    resource! {
        TenantLeaf {
            table "scoped_managed_tenant_leaves";
            multitenancy { strategy: attribute; attribute: org; }
            attributes { id: Uuid [pk]; org: String; folder_id: Uuid; }
            actions {
                create create { primary; accept [folder_id]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}
pub use tenant_leaf::TenantLeaf;

fn fail_write(_: &mut ash_core::RawRecord) -> ash_core::Result<()> {
    Err(Error::Invalid("refuse after persistence".into()))
}

fn fail_destroy(_: &mut ash_core::RawRecord) -> ash_core::Result<()> {
    Err(Error::Invalid("refuse after deletion".into()))
}

pub mod blocker {
    use super::*;
    resource! {
        Blocker {
            table "scoped_managed_blockers";
            attributes { id: Uuid [pk]; folder_id: Uuid; }
            actions {
                create create { primary; accept [folder_id]; }
                read read { primary; }
                destroy destroy { primary; after_action fail_destroy; }
            }
        }
    }
}
pub use blocker::Blocker;

pub const RESOURCES: &[&ash_core::ResourceDef] = &[
    &Document::DEF,
    &Remark::DEF,
    &KeyedRemark::DEF,
    &CompositeRemark::DEF,
    &Article::DEF,
    &Label::DEF,
    &Link::DEF,
    &GlobalLink::DEF,
    &ReadonlyLink::DEF,
    &Folder::DEF,
    &Leaf::DEF,
    &TenantLeaf::DEF,
    &Blocker::DEF,
];

fn key() -> String {
    Uuid::new_v4().to_string()
}
fn actor(id: Uuid, role: &str) -> Actor {
    Actor::new(id).with_role(role)
}
fn input(id: Uuid) -> FieldMap {
    fields! { "id" => id }
}
fn not_found<T: std::fmt::Debug>(result: ash_core::Result<T>) {
    let error = result.unwrap_err();
    assert!(matches!(error, Error::NotFound), "{error:?}");
    assert_eq!(error.to_string(), "not found");
}
fn scopes<D>(runtime: &Runtime<D>, system: bool) -> (Context<D>, Context<D>, Uuid) {
    let owner = Uuid::new_v4();
    let runtime = if system {
        runtime.system()
    } else {
        runtime.clone()
    };
    (
        runtime.bind(Some(actor(owner, "linker")), Some(key())),
        runtime.bind(Some(actor(owner, "linker")), Some(key())),
        owner,
    )
}
async fn document<D: DataLayer>(ctx: &Context<D>, owner: Uuid) -> Document {
    Document::create(ctx)
        .code(key())
        .owner(owner)
        .title("document")
        .await
        .unwrap()
}
async fn label<D: DataLayer>(ctx: &Context<D>, owner: Uuid) -> Label {
    Label::create(ctx).name(key()).owner(owner).await.unwrap()
}

pub async fn belongs_to_ids_resolve_in_scope<D: TransactionSupport + 'static>(runtime: Runtime<D>) {
    for system in [false, true] {
        let (a, b, owner) = scopes(&runtime, system);
        let own = document(&a, owner).await;
        let foreign = document(&b, owner).await;
        let related = Remark::create(&a)
            .body("same tenant")
            .manage_relationship_one(
                "document",
                fields! {"id" => own.id.to_string()},
                ManagedRelType::Append,
            )
            .await
            .unwrap();
        assert_eq!(related.document_id, Some(own.id));
        for id in [foreign.id, Uuid::new_v4()] {
            not_found(
                Remark::create(&a)
                    .body("unresolved")
                    .manage_relationship_one("document", input(id), ManagedRelType::Append)
                    .await,
            );
        }
        for id in [
            Value::String("invalid".into()),
            Value::Bool(true),
            Value::Null,
        ] {
            for composite in [false, true] {
                if composite {
                    not_found(
                        CompositeRemark::create(&a)
                            .body("malformed")
                            .manage_relationship_one(
                                "document",
                                fields! {"id" => id.clone()},
                                ManagedRelType::Append,
                            )
                            .await,
                    );
                } else {
                    not_found(
                        Remark::create(&a)
                            .body("malformed")
                            .manage_relationship_one(
                                "document",
                                fields! {"id" => id.clone()},
                                ManagedRelType::Append,
                            )
                            .await,
                    );
                }
            }
        }
        assert_eq!(Remark::query(&a).all().await.unwrap().len(), 1);
        let related = CompositeRemark::create(&a)
            .body("same tenant")
            .manage_relationship_one(
                "document",
                fields! {"id" => own.id.to_string()},
                ManagedRelType::Append,
            )
            .await
            .unwrap();
        assert_eq!(related.document_org.as_deref(), a.tenant());
        assert_eq!(related.document_code.as_deref(), Some(own.code.as_str()));
        not_found(
            CompositeRemark::create(&a)
                .body("unresolved")
                .manage_relationship_one("document", input(foreign.id), ManagedRelType::Append)
                .await,
        );
        assert_eq!(CompositeRemark::query(&a).all().await.unwrap().len(), 1);
    }
}

pub async fn relationship_resolution_applies_read_policies_and_preparations<
    D: TransactionSupport + 'static,
>(
    runtime: Runtime<D>,
) {
    let (a, _, owner) = scopes(&runtime, false);
    let hidden = document(&a, Uuid::new_v4()).await;
    let filtered = Document::create(&a)
        .code(key())
        .owner(owner)
        .title("filtered")
        .visible(false)
        .await
        .unwrap();
    for id in [hidden.id, filtered.id] {
        not_found(
            Remark::create(&a)
                .body("unreadable")
                .manage_relationship_one("document", input(id), ManagedRelType::Append)
                .await,
        );
        not_found(
            CompositeRemark::create(&a)
                .body("unreadable")
                .manage_relationship_one("document", input(id), ManagedRelType::Append)
                .await,
        );
    }
    let article = Article::create(&a).title("article").await.unwrap();
    let hidden = label(&a, Uuid::new_v4()).await;
    let filtered = Label::create(&a)
        .name(key())
        .owner(owner)
        .visible(false)
        .await
        .unwrap();
    for mode in [ManagedRelType::Append, ManagedRelType::DirectControl] {
        for id in [hidden.id, filtered.id] {
            not_found(
                Article::update(&a, article.id)
                    .manage_relationship("labels", vec![input(id)], mode)
                    .await,
            );
        }
    }
    let system = runtime
        .system()
        .bind(a.actor().cloned(), a.tenant().map(str::to_owned));
    Article::update(&system, article.id)
        .manage_relationship("labels", vec![input(hidden.id)], ManagedRelType::Append)
        .await
        .unwrap();
    not_found(
        Article::update(&system, article.id)
            .manage_relationship("labels", vec![input(filtered.id)], ManagedRelType::Append)
            .await,
    );
}

pub async fn many_to_many_ids_resolve_before_linking<D: TransactionSupport + 'static>(
    runtime: Runtime<D>,
) {
    for system in [false, true] {
        let (a, b, owner) = scopes(&runtime, system);
        let foreign = label(&b, owner).await;
        for relationship in ["labels", "global_labels"] {
            for mode in [ManagedRelType::Append, ManagedRelType::DirectControl] {
                let article = Article::create(&a).title("article").await.unwrap();
                let own = label(&a, owner).await;
                Article::update(&a, article.id)
                    .manage_relationship(
                        relationship,
                        vec![fields! {"id" => own.id.to_string()}],
                        mode,
                    )
                    .await
                    .unwrap();
                for id in [
                    Value::String("invalid".into()),
                    Value::Bool(true),
                    Value::Null,
                ] {
                    not_found(
                        Article::update(&a, article.id)
                            .manage_relationship(relationship, vec![fields! {"id" => id}], mode)
                            .await,
                    );
                }
                not_found(
                    Article::update(&a, article.id)
                        .title("must roll back")
                        .manage_relationship(relationship, vec![input(foreign.id)], mode)
                        .await,
                );
                assert_eq!(Article::get(&a, article.id).await.unwrap().title, "article");
                let through = if relationship == "labels" {
                    &Link::DEF
                } else {
                    &GlobalLink::DEF
                };
                let rows = a
                    .access()
                    .read(
                        through,
                        "read",
                        &FieldMap::new(),
                        ash_core::ReadRequest {
                            filter: Some(Filter::eq("article_id", article.id)),
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].get("label_id"), Some(&Value::Uuid(own.id)));
                if relationship == "labels" {
                    assert_eq!(rows[0].get("org").and_then(Value::as_str), a.tenant());
                }
                Article::update(&a, article.id)
                    .manage_relationship(
                        relationship,
                        Vec::<FieldMap>::new(),
                        ManagedRelType::DirectControl,
                    )
                    .await
                    .unwrap();
            }
        }
    }
}

pub async fn join_actions_enforce_create_and_destroy_policies<D: TransactionSupport + 'static>(
    runtime: Runtime<D>,
) {
    let (a, _, owner) = scopes(&runtime, false);
    let denied = runtime.bind(Some(actor(owner, "reader")), a.tenant().map(str::to_owned));
    let article = Article::create(&a).title("article").await.unwrap();
    let own = label(&a, owner).await;
    assert!(matches!(
        Article::update(&denied, article.id)
            .title("denied")
            .manage_relationship("labels", vec![input(own.id)], ManagedRelType::Append)
            .await,
        Err(Error::Forbidden)
    ));
    assert!(Link::query(&a).all().await.unwrap().is_empty());
    let error = Article::update(&a, article.id)
        .manage_relationship(
            "readonly_labels",
            vec![input(own.id)],
            ManagedRelType::Append,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Invalid(_)));
    assert!(ReadonlyLink::query(&a).all().await.unwrap().is_empty());
    Article::update(&a, article.id)
        .manage_relationship("labels", vec![input(own.id)], ManagedRelType::Append)
        .await
        .unwrap();
    assert!(matches!(
        Article::update(&denied, article.id)
            .title("denied")
            .manage_relationship(
                "labels",
                Vec::<FieldMap>::new(),
                ManagedRelType::DirectControl
            )
            .await,
        Err(Error::Forbidden)
    ));
    assert_eq!(Article::get(&a, article.id).await.unwrap().title, "article");
    assert_eq!(Link::query(&a).all().await.unwrap().len(), 1);
    // A many-to-many-only cascade must take the record path and honor join actions too.
    assert!(matches!(
        ash_core::destroy_existing(&denied, "destroy", article.clone()).await,
        Err(Error::Forbidden)
    ));
    assert!(Article::get(&a, article.id).await.is_ok());
    article.destroy(&a).await.unwrap();
    assert!(Link::query(&a).all().await.unwrap().is_empty());
}

pub async fn has_many_direct_control_keeps_foreign_children_untouched<
    D: TransactionSupport + 'static,
>(
    runtime: Runtime<D>,
) {
    let (a, b, owner) = scopes(&runtime, false);
    let own = document(&a, owner).await;
    let foreign = document(&b, owner).await;
    let child = Remark::create(&b)
        .document_id(foreign.id)
        .body("foreign")
        .await
        .unwrap();
    let mismatched = Remark::create(&b)
        .document_id(own.id)
        .body("foreign parent key")
        .await
        .unwrap();
    Document::update(&a, own.id)
        .manage_relationship(
            "remarks",
            vec![fields! {"id" => child.id, "body" => "new"}],
            ManagedRelType::DirectControl,
        )
        .await
        .unwrap();
    assert_eq!(Remark::get(&b, child.id).await.unwrap().body, "foreign");
    assert_eq!(
        Remark::get(&b, mismatched.id).await.unwrap().body,
        "foreign parent key"
    );
    let created = Remark::query(&a).one().await.unwrap();
    assert_ne!(created.id, child.id);
    assert_eq!(created.document_id, Some(own.id));
    Document::update(&a, own.id)
        .manage_relationship(
            "remarks",
            vec![fields! {"id" => created.id.to_string(), "body" => "updated"}],
            ManagedRelType::DirectControl,
        )
        .await
        .unwrap();
    assert_eq!(Remark::get(&a, created.id).await.unwrap().body, "updated");
}

pub async fn rejected_parent_destroy_preserves_cascade_children<D: TransactionSupport + 'static>(
    runtime: Runtime<D>,
) {
    for system in [false, true] {
        let (a, b, _) = scopes(&runtime, system);
        let folder = Folder::create(&b).name("foreign").await.unwrap();
        for _ in 0..2 {
            Leaf::create(&b)
                .folder_id(folder.id)
                .name("leaf")
                .await
                .unwrap();
        }
        not_found(ash_core::destroy_existing(&a, "destroy", folder.clone()).await);
        assert!(Folder::get(&b, folder.id).await.is_ok());
        assert_eq!(
            Leaf::query(&b)
                .filter(Filter::eq("folder_id", folder.id))
                .all()
                .await
                .unwrap()
                .len(),
            2
        );
        let own_child = TenantLeaf::create(&b).folder_id(folder.id).await.unwrap();
        let other_child = TenantLeaf::create(&a).folder_id(folder.id).await.unwrap();
        folder.destroy(&b).await.unwrap();
        assert!(TenantLeaf::get(&b, own_child.id).await.is_err());
        assert!(TenantLeaf::get(&a, other_child.id).await.is_ok());
        assert!(
            Leaf::query(&b)
                .filter(Filter::eq("folder_id", folder.id))
                .all()
                .await
                .unwrap()
                .is_empty()
        );
    }
}

pub async fn later_cascade_failure_rolls_back_earlier_deletes<D: TransactionSupport + 'static>(
    runtime: Runtime<D>,
) {
    let notified = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let heard = notified.clone();
    let runtime = runtime.with_notifier(std::sync::Arc::new(ash_core::SyncFnNotifier::new(
        "committed",
        move |_| {
            heard.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        },
    )));
    let (a, _, _) = scopes(&runtime, false);
    let folder = Folder::create(&a).name("folder").await.unwrap();
    let leaf = Leaf::create(&a)
        .folder_id(folder.id)
        .name("deleted first")
        .await
        .unwrap();
    let blocker = Blocker::create(&a).folder_id(folder.id).await.unwrap();
    notified.store(0, std::sync::atomic::Ordering::SeqCst);
    let error = folder.clone().destroy(&a).await.unwrap_err();
    assert_eq!(error.to_string(), "refuse after deletion");
    assert!(Folder::get(&a, folder.id).await.is_ok());
    assert!(Leaf::get(&a, leaf.id).await.is_ok());
    assert!(Blocker::get(&a, blocker.id).await.is_ok());
    let result = ash_core::bulk_destroy::<Folder, _, _>(
        &a,
        "destroy",
        &[folder.id],
        ash_core::BulkDestroyOptions::new().transaction(ash_core::BulkTransaction::Off),
    )
    .await;
    assert!(result.is_err());
    assert!(Folder::get(&a, folder.id).await.is_ok());
    assert!(Leaf::get(&a, leaf.id).await.is_ok());
    assert!(Blocker::get(&a, blocker.id).await.is_ok());
    assert_eq!(notified.load(std::sync::atomic::Ordering::SeqCst), 0);
}

pub async fn later_relationship_failure_rolls_back_parent_and_joins<
    D: TransactionSupport + 'static,
>(
    runtime: Runtime<D>,
) {
    let (a, b, owner) = scopes(&runtime, false);
    let article = Article::create(&a).title("original").await.unwrap();
    let own = label(&a, owner).await;
    let foreign = label(&b, owner).await;
    for mode in [ManagedRelType::Append, ManagedRelType::DirectControl] {
        not_found(
            Article::update(&a, article.id)
                .title("changed")
                .manage_relationship("labels", vec![input(own.id), input(foreign.id)], mode)
                .await,
        );
        assert_eq!(
            Article::get(&a, article.id).await.unwrap().title,
            "original"
        );
        assert!(Link::query(&a).all().await.unwrap().is_empty());
    }
}

pub async fn bulk_managed_rows_keep_preparation_inside_the_transaction<
    D: TransactionSupport + 'static,
>(
    runtime: Runtime<D>,
) {
    let (a, b, owner) = scopes(&runtime, false);
    let code = key();
    let new_document = fields! { "code" => code.clone(), "owner" => owner, "title" => "new" };
    let result = ash_core::bulk_create::<Remark, _, _, _>(
        &a,
        "create_with_document",
        [fields! { "body" => "new", "document" => Value::Map(new_document) }],
        ash_core::BulkCreateOptions::new().transaction(ash_core::BulkTransaction::Off),
    )
    .await;
    assert_eq!(result.unwrap_err().to_string(), "refuse after persistence");
    assert!(
        Document::query(&a)
            .filter(Filter::eq("code", code))
            .all()
            .await
            .unwrap()
            .is_empty()
    );
    assert!(Remark::query(&a).all().await.unwrap().is_empty());

    let article = Article::create(&a).title("original").await.unwrap();
    let own = label(&a, owner).await;
    let foreign = label(&b, owner).await;
    let result = ash_core::bulk_update::<Article, _, _, _>(&a, "manage_labels",
        [(article.clone(), fields! { "title" => "changed", "labels" => Value::Array(vec![Value::Map(input(own.id)), Value::Map(input(foreign.id))]) })],
        ash_core::BulkUpdateOptions::new().transaction(ash_core::BulkTransaction::Off)).await;
    not_found(result);
    assert_eq!(
        Article::get(&a, article.id).await.unwrap().title,
        "original"
    );
    assert!(Link::query(&a).all().await.unwrap().is_empty());
}

pub async fn managed_creates_preserve_explicit_primary_keys<D: TransactionSupport + 'static>(
    runtime: Runtime<D>,
) {
    let (a, _, owner) = scopes(&runtime, false);
    let document = document(&a, owner).await;
    for mode in [ManagedRelType::Create, ManagedRelType::Append] {
        let id = key();
        Document::update(&a, document.id)
            .manage_relationship(
                "keyed_remarks",
                vec![fields! {"id" => id.clone(), "body" => "created"}],
                mode,
            )
            .await
            .unwrap();
        let child = KeyedRemark::get(&a, id.clone()).await.unwrap();
        assert_eq!(child.id, id);
        assert_eq!(child.document_id, document.id);
        assert_eq!(child.org.as_str(), a.tenant().unwrap());
    }
}

macro_rules! cases {
    ($setup:ident) => {
        #[tokio::test]
        async fn managed_creates_preserve_explicit_primary_keys() {
            support::managed_creates_preserve_explicit_primary_keys($setup().await).await;
        }

        #[tokio::test]
        async fn bulk_managed_rows_keep_preparation_inside_the_transaction() {
            support::bulk_managed_rows_keep_preparation_inside_the_transaction($setup().await)
                .await;
        }

        #[tokio::test]
        async fn belongs_to_ids_resolve_in_scope() {
            support::belongs_to_ids_resolve_in_scope($setup().await).await;
        }
        #[tokio::test]
        async fn relationship_resolution_applies_read_policies_and_preparations() {
            support::relationship_resolution_applies_read_policies_and_preparations($setup().await)
                .await;
        }
        #[tokio::test]
        async fn many_to_many_ids_resolve_before_linking() {
            support::many_to_many_ids_resolve_before_linking($setup().await).await;
        }
        #[tokio::test]
        async fn join_actions_enforce_create_and_destroy_policies() {
            support::join_actions_enforce_create_and_destroy_policies($setup().await).await;
        }
        #[tokio::test]
        async fn has_many_direct_control_keeps_foreign_children_untouched() {
            support::has_many_direct_control_keeps_foreign_children_untouched($setup().await).await;
        }
        #[tokio::test]
        async fn rejected_parent_destroy_preserves_cascade_children() {
            support::rejected_parent_destroy_preserves_cascade_children($setup().await).await;
        }
        #[tokio::test]
        async fn later_cascade_failure_rolls_back_earlier_deletes() {
            support::later_cascade_failure_rolls_back_earlier_deletes($setup().await).await;
        }
        #[tokio::test]
        async fn later_relationship_failure_rolls_back_parent_and_joins() {
            support::later_relationship_failure_rolls_back_parent_and_joins($setup().await).await;
        }
    };
}
pub(crate) use cases;
