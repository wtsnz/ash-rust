use ash_core::{Decimal, resource};

resource! {
    /// What a container declares to customs. Stored as JSON on the container.
    embedded Manifest {
        attributes {
            shipper: String;
            description: String;
            pieces: i64;
            declared_value: Decimal;
        }

        actions {
            create declare {
                primary;
                accept [shipper, description, pieces, declared_value];
                validate present(description);
                validate numericality(pieces, min: 1);
            }
        }
    }
}
