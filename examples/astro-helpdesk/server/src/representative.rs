use ash_core::resource;
use uuid::Uuid;

resource! {
    resource Representative;
    table "representatives";

    attributes {
        id: Uuid [pk],
        name: String,
        email: String,
        role: String,
    }

    actions {
        create create {
            primary
            accept [name, email, role]
            validate present(name);
            validate present(email);
        }

        read read {
            primary
        }
    }
}
