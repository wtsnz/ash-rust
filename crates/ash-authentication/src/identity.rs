use ash_core::{Actor, Context, Runtime};

/// Actor and resolved tenant authenticated together. Private fields prevent a handler
/// from accidentally retaining the actor while replacing its authenticated tenant.
#[derive(Clone, Debug)]
pub struct BoundIdentity {
    actor: Actor,
    tenant: Option<String>,
}

impl BoundIdentity {
    pub(crate) fn new(actor: Actor, tenant: Option<String>) -> Self {
        Self { actor, tenant }
    }
    pub fn actor(&self) -> &Actor {
        &self.actor
    }
    pub fn tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
    }
    pub fn bind<D>(&self, runtime: &Runtime<D>) -> Context<D> {
        runtime.bind(Some(self.actor.clone()), self.tenant.clone())
    }
}
