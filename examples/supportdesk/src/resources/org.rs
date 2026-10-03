use ash_core::resource;
use uuid::Uuid;

resource! {
    /// A customer of the support desk. Its `slug` is the tenant every other resource
    /// belongs to.
    Org {
        table "orgs";

        attributes {
            id: Uuid [pk];
            name: String;
            slug: String;
        }

        identities {
            identity unique_slug: [slug];
        }

        actions {
            read read {
                primary;
            }

            create seed {
                accept [id, name, slug];
            }
        }
    }
}
