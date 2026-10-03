use ash_core::{Context, DataLayer, Filter, Resource, ResourceExt};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

pub mod account_mod {
    use super::membership_mod::Membership;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Account {
            table "accounts";

            attributes {
                id: Uuid [pk];
                tenant_id: Uuid;
                code: String;
            }

            relationships {
                has_many memberships: Membership
                    [fk: [tenant_id, code], references: [tenant_id, code], on_delete: cascade];
            }

            aggregates {
                member_count: Option<i64> = count(memberships);
            }

            identities {
                identity tenant_code: [tenant_id, code];
            }

            actions {
                create create { primary; accept [tenant_id, code]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}

pub mod membership_mod {
    use super::account_mod::Account;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Membership {
            table "memberships";

            attributes {
                id: Uuid [pk];
                tenant_id: Uuid;
                code: String;
                label: String;
            }

            relationships {
                belongs_to account: Account
                    [fk: [tenant_id, code], references: [tenant_id, code]];
            }

            actions {
                create create { primary; accept [tenant_id, code, label]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}

use account_mod::Account;
use membership_mod::Membership;

/// Two accounts in one tenant, told apart only by `code`, each with one membership.
async fn scenario<D: DataLayer>(ctx: Context<D>) {
    let tenant = Uuid::from_u128(7);
    for code in ["a", "b"] {
        Account::create(&ctx).tenant_id(tenant).code(code).await.unwrap();
        Membership::create(&ctx)
            .tenant_id(tenant)
            .code(code)
            .label(format!("for {code}"))
            .await
            .unwrap();
    }

    for membership in Membership::query(&ctx).load_rel(Membership::account).all().await.unwrap() {
        let account = membership.account.loaded().unwrap().as_ref().unwrap();
        assert_eq!(account.code, membership.code, "belongs_to must match every key column");
    }

    let accounts = Account::query(&ctx)
        .load_rel(Account::memberships)
        .load_aggregate(Account::member_count)
        .all()
        .await
        .unwrap();
    for account in &accounts {
        let labels: Vec<&str> = account
            .memberships
            .loaded()
            .unwrap()
            .iter()
            .map(|m| m.label.as_str())
            .collect();
        assert_eq!(labels, [format!("for {}", account.code)], "has_many by full key");
        assert_eq!(account.member_count, Some(1));
    }

    let matched: Vec<String> = Account::query(&ctx)
        .filter(Filter::related("memberships", Filter::eq("label", "for a")))
        .all()
        .await
        .unwrap()
        .into_iter()
        .map(|a| a.code)
        .collect();
    assert_eq!(matched, ["a"]);

    // Cascading from `a` must not touch `b`'s membership, which shares the tenant.
    let a = accounts.into_iter().find(|a| a.code == "a").unwrap();
    a.destroy(&ctx).await.unwrap();
    let left: Vec<String> = Membership::query(&ctx)
        .all()
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.code)
        .collect();
    assert_eq!(left, ["b"]);
}

#[tokio::test]
async fn composite_keys_in_memory() {
    scenario(Context::new(Memory::new())).await;
}

#[tokio::test]
async fn composite_keys_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite
        .install(&[&Account::DEF, &Membership::DEF])
        .await
        .unwrap();
    scenario(Context::new(sqlite)).await;
}
