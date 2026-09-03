use super::member::WorkspaceMember;
use ash_core::resource;
use uuid::Uuid;

resource! {
    resource Workspace;
    table "workspaces";

    attributes {
        id: Uuid [pk],
        name: String,
        slug: String,
        owner_id: Uuid,
    }

    relationships {
        has_many members: Vec<WorkspaceMember> [fk: "workspace_id"],
    }

    aggregates {
        member_count: Option<i64> = count(members),
        has_members: Option<bool> = exists(members),
    }

    actions {
        create create {
            accept {
                name: String,
                slug: String,
            }
            validate present(name);
            validate present(slug);
            change relate_actor(owner_id);
        }

        read read {
            primary
        }
    }

    policies {
        policy action(create) {
            authorize_if actor_present
        }
        policy action_type(read) {
            authorize_if always
        }
    }
}
