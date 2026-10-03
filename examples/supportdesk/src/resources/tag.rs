use ash_core::resource;
use uuid::Uuid;

resource! {
    /// A label an org files its tickets under.
    Tag {
        table "tags";

        multitenancy {
            strategy: attribute;
            attribute: org;
        }

        attributes {
            id: Uuid [pk];
            org: String;
            name: String;
        }

        identities {
            identity unique_name: [org, name];
        }

        actions {
            read read {
                primary;
            }

            create seed {
                accept [id, org, name];
            }
        }
    }
}
