use super::user::User;
use super::workspace::Workspace;
use ash_core::resource;
use uuid::Uuid;

resource! {
    resource WorkspaceMember;
    table "workspace_members";

    attributes {
        id: Uuid [pk],
        workspace_id: Uuid,
        user_id: Uuid,
        role: String,
    }

    relationships {
        belongs_to workspace: Workspace [fk: workspace_id],
        belongs_to user: User [fk: user_id],
    }

    actions {
        create add {
            accept [workspace_id, user_id, role];
            validate present(role);
            validate one_of(role, ["admin", "member", "guest"]);
        }

        read read {
            primary
        }

        destroy destroy {}
    }

    policies {
        policy always {
            authorize_if always
        }
    }
}
