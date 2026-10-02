//! How the schema names things, as AshGraphql (through Absinthe's language conventions)
//! names them: fields and arguments in camelCase, types and inputs in PascalCase, enum
//! values in UPPER_SNAKE_CASE.

/// `call_sign` → `callSign`.
pub fn camel(name: &str) -> String {
    let pascal = pascal(name);
    let mut chars = pascal.chars();
    match chars.next() {
        Some(first) => first.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// `call_sign` → `CallSign`; `ServiceZone` stays `ServiceZone`.
pub fn pascal(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut upper = true;
    for ch in name.chars() {
        if ch == '_' || ch == '-' {
            upper = true;
        } else if upper {
            out.push(ch.to_ascii_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// `call_sign` → `CALL_SIGN`; `ServiceZone` → `SERVICE_ZONE`.
pub fn upper_snake(name: &str) -> String {
    snake(name).to_ascii_uppercase()
}

/// `ServiceZone` → `service_zone`; `cabCreated` → `cab_created`.
pub fn snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    out
}

/// `Ticket` → `Tickets`; `Box` → `Boxes`.
pub fn plural(name: &str) -> String {
    let es = ["s", "x", "z", "ch", "sh"].iter().any(|end| name.ends_with(end));
    format!("{name}{}", if es { "es" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_absinthe() {
        assert_eq!(camel("call_sign"), "callSign");
        assert_eq!(camel("id"), "id");
        assert_eq!(camel("cabin_temp_c"), "cabinTempC");
        assert_eq!(pascal("host_event"), "HostEvent");
        assert_eq!(pascal("ServiceZone"), "ServiceZone");
        assert_eq!(upper_snake("trips_completed"), "TRIPS_COMPLETED");
        assert_eq!(snake("ServiceZone"), "service_zone");
        assert_eq!(snake("serviceZoneCreated"), "service_zone_created");
        assert_eq!(plural("Trip"), "Trips");
    }
}
