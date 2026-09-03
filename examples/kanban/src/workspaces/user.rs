use ash_core::resource;
use uuid::Uuid;

resource! {
    resource User;
    table "users";

    attributes {
        id: Uuid [pk],
        name: String,
        email: String,
    }

    actions {
        create register {
            accept {
                name: String,
                email: String,
            }
            validate present(name);
            validate present(email);
        }

        read read {
            primary
        }
    }

    policies {
        policy always {
            authorize_if always
        }
    }
}
