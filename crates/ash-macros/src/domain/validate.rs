use super::ast::DomainDefinition;
use std::collections::HashSet;
use syn::{Error, Ident};

pub fn validate(def: &DomainDefinition) -> Vec<Error> {
    let mut errors = Vec::new();
    check_unique_idents(
        def.resources.iter().map(|r| &r.resource),
        "resource",
        &mut errors,
    );
    for res in &def.resources {
        check_unique_idents(
            res.interfaces.iter().map(|i| &i.fn_name),
            "code interface",
            &mut errors,
        );
    }
    errors
}

fn check_unique_idents<'a>(
    idents: impl IntoIterator<Item = &'a Ident>,
    kind: &str,
    errors: &mut Vec<Error>,
) {
    let mut seen = HashSet::new();
    for ident in idents {
        let name = ident.to_string();
        if !seen.insert(name.clone()) {
            errors.push(Error::new_spanned(
                ident,
                format!("duplicate {kind} `{name}`"),
            ));
        }
    }
}
