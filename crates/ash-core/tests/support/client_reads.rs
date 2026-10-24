use ash_core::{Actor, Context, DataLayer, Resource, Runtime, resource};
use uuid::Uuid;

pub mod tag {
    use super::*;
    resource! {
        Tag {
            table "client_read_tags";
            attributes { id: Uuid [pk]; name: String; active: bool [default: true]; }
            actions {
                create create { primary; accept [name, active]; }
                read read { primary; prepare filter(active == true); }
            }
        }
    }
}

pub mod post_tag {
    use super::*;
    resource! {
        PostTag {
            table "client_read_post_tags";
            actor { role: String; }
            attributes {
                id: Uuid [pk]; post_id: Uuid; tag_id: Uuid;
                public: bool; enabled: bool [default: true];
            }
            policies {
                policy action_type(create) { authorize_if always; }
                policy action_type(read) {
                    authorize_if eq(public, true);
                    authorize_if actor_attribute_equals(role, "admin");
                }
            }
            actions {
                create create { primary; accept [post_id, tag_id, public, enabled]; }
                read read { primary; prepare filter(enabled == true); }
            }
        }
    }
}

pub mod post {
    use super::*;
    use super::{post_tag::PostTag, tag::Tag};
    resource! {
        Post {
            table "client_read_posts";
            attributes { id: Uuid [pk]; title: String; }
            relationships {
                many_to_many tags: Tag [through: PostTag, source_fk: post_id, dest_fk: tag_id];
            }
            aggregates {
                tag_count: Option<i64> = count(tags);
                has_tags: Option<bool> = exists(tags);
                first_tag: Option<String> = first(tags, name);
            }
            actions {
                create create { primary; accept [title]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}

pub mod budget {
    use super::*;
    resource! {
        Budget {
            table "client_read_budgets";
            attributes { id: Uuid [pk]; name: String; }
            relationships { has_many payrolls: super::payroll::Payroll [fk: budget_id]; }
            actions {
                create create { primary; accept [name]; }
                read read { primary; }
            }
        }
    }
}

// Core consumes the attribute-named read argument as a client comparison.
#[allow(deprecated)]
pub mod payroll {
    use super::*;
    resource! {
        Payroll {
            table "client_read_payrolls";
            actor { role: String; }
            attributes {
                id: Uuid [pk]; name: String; salary: Option<i64>;
                lookup: String; budget_id: Option<Uuid>;
            }
            identities { identity by_lookup: [lookup]; }
            relationships { belongs_to budget: super::budget::Budget [fk: budget_id]; }
            calculations { doubled: i64 = coalesce(salary, 0) * 2; }
            field_policies {
                field salary { authorize_if actor_attribute_equals(role, "admin"); }
                field lookup { authorize_if actor_attribute_equals(role, "admin"); }
            }
            actions {
                create create { primary; accept [name, salary, lookup, budget_id]; }
                read read {
                    primary;
                    pagination keyset: true, required: false;
                    prepare filter(salary >= 100);
                    prepare sort(salary, desc);
                }
                read by_salary { argument salary: i64; }
                update update { primary; accept [name]; }
            }
        }
    }
}

pub use {budget::Budget, payroll::Payroll, post::Post, post_tag::PostTag, tag::Tag};
pub const RESOURCES: &[&ash_core::ResourceDef] = &[
    &Tag::DEF,
    &PostTag::DEF,
    &Post::DEF,
    &Budget::DEF,
    &Payroll::DEF,
];

pub fn admin() -> Actor {
    Actor::new(Uuid::from_u128(1)).with_role("admin")
}
pub fn member() -> Actor {
    Actor::new(Uuid::from_u128(2)).with_role("member")
}

pub async fn seed<D: DataLayer>(runtime: &Runtime<D>) -> Uuid {
    let ctx = runtime.bind(Some(admin()), None);
    for (title, public, enabled, active) in [
        ("private", false, true, true),
        ("public", true, true, true),
        ("disabled", true, false, true),
        ("inactive", true, true, false),
    ] {
        let post = Post::create(&ctx).title(title).await.unwrap();
        let tag = Tag::create(&ctx).name(title).active(active).await.unwrap();
        PostTag::create(&ctx)
            .post_id(post.id)
            .tag_id(tag.id)
            .public(public)
            .enabled(enabled)
            .await
            .unwrap();
    }
    let budget = Budget::create(&ctx).name("engineering").await.unwrap();
    for (name, salary) in [("low", 100), ("middle", 200), ("high", 300)] {
        Payroll::create(&ctx)
            .name(name)
            .salary(Some(salary))
            .lookup(format!("key-{name}"))
            .budget_id(Some(budget.id))
            .await
            .unwrap();
    }
    budget.id
}

pub fn reader<D>(runtime: &Runtime<D>, actor: Actor) -> Context<D> {
    runtime.bind(Some(actor), None)
}
