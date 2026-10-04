use ash_authentication::authentication;
use ash_core::CiString;
use uuid::Uuid;

use crate::types::Role;

#[authentication]
resource! {
    /// Someone who signs in: dispatchers, captains, customs officers, shippers, and
    /// the port authority. Their `role` and `line` travel on the actor into policies.
    CrewMember {
        table "crew_members";
        timestamps;

        multitenancy {
            strategy: attribute;
            attribute: line;
        }

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            line: String;
            email: CiString;
            name: String;
            role: Role;
            hashed_password: Option<String>;
        }

        identities {
            identity unique_email: [email], message: "that address already has an account";
        }

        authentication {
            strategy password {
                identity_field: email;
                hashed_password_field: hashed_password;
                min_password_length: 10;
                require_confirmation: false;
            }

            strategy tokens {
                token_lifetime_secs: 3600;
            }
        }

        actions {
            // #[authentication] adds the password argument and hashing to this action.
            create register_with_password {
                accept [email, name, role];
                validate string_length(name, min: 2);
            }

            read read {
                primary;
            }

            update reassign {
                accept [role];
            }
        }

        policies {
            bypass {
                authorize_if actor_eq(role = "port_authority");
            }
            policy action_type(read) {
                authorize_if actor_present;
            }
            policy action(register_with_password) | action(reassign) {
                authorize_if actor_eq(role = "dispatcher");
            }
        }
    }
}
