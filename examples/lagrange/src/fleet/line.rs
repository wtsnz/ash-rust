use ash_core::{CiString, resource};
use uuid::Uuid;

resource! {
    /// A shipping company. Its `slug` is the tenant key on everything the line owns.
    ShippingLine {
        table "shipping_lines";
        timestamps;

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            slug: CiString;
            name: String;
            dispatch_email: CiString;
        }

        identities {
            identity unique_slug: [slug], message: "that line is already registered";
        }

        actions {
            create register {
                primary;
                accept [slug, name, dispatch_email];
                validate string_length(name, min: 2);
            }

            read read {
                primary;
            }
        }

        policies {
            policy action_type(read) {
                authorize_if always;
            }
            policy action(register) {
                authorize_if actor_eq(role = "port_authority");
            }
        }
    }
}
