use ash_archival::archival;
use ash_core::{Binary, ChangeContext, CiString, Result, Value};
use uuid::Uuid;

use super::contract::Contract;
use super::manifest::Manifest;
use crate::types::HazmatClass;

/// Marks dangerous goods so queries and aggregates can find them without decoding the
/// hazmat class.
pub fn mark_hazardous(ctx: &mut ChangeContext<'_>) -> Result<()> {
    let hazardous = matches!(ctx.fields.get("hazmat"), Some(Value::String(_)));
    ctx.fields.insert("hazardous".into(), Value::Bool(hazardous));
    Ok(())
}

#[archival]
resource! {
    /// A standard cargo container. Its manifest is for the shipper and customs only,
    /// and dangerous goods must be sealed by customs before the voyage can clear.
    Container {
        table "containers";
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
            code: CiString;
            contract_id: Uuid;
            mass_tonnes: i64;
            hazmat: Option<HazmatClass>;
            hazardous: bool [default: false];
            reefer: bool [default: false];
            manifest: Option<Manifest>;
            seal: Option<Binary>;
            sealed: bool [default: false];
            version: i64 [version];
        }

        identities {
            identity live_code: [code], where: "archived_at IS NULL";
        }

        checks {
            check positive_mass: "mass_tonnes > 0";
            check sealed_has_seal: "sealed = (seal IS NOT NULL)";
        }

        indexes {
            index by_contract: [contract_id];
            index hazardous_by_contract: [contract_id], where: "hazardous";
        }

        relationships {
            belongs_to contract: Contract [fk: contract_id, on_delete: restrict];
        }

        field_policies {
            field manifest {
                authorize_if actor_eq(role = "shipper");
                authorize_if actor_eq(role = "customs");
                authorize_if actor_eq(role = "port_authority");
            }
            field seal {
                authorize_if actor_eq(role = "customs");
                authorize_if actor_eq(role = "port_authority");
            }
        }

        actions {
            create pack {
                primary;
                accept [code, contract_id, mass_tonnes, hazmat, reefer, manifest];
                change func(mark_hazardous);
                validate numericality(mass_tonnes, min: 1, max: 40);
            }

            read read {
                primary;
            }

            read scrapped {
                prepare filter(!archived_at.is_nil());
            }

            update seal {
                accept [seal];
                change set(sealed = true);
            }

            destroy scrap {
                primary;
            }
        }

        policies {
            bypass {
                authorize_if actor_eq(role = "port_authority");
            }
            policy action_type(read) {
                authorize_if actor_present;
            }
            policy action(pack) | action(scrap) {
                authorize_if actor_eq(role = "shipper");
                authorize_if actor_eq(role = "dispatcher");
            }
            policy action(seal) {
                authorize_if actor_eq(role = "customs");
            }
        }
    }
}
