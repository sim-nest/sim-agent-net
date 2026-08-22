use crate::{CredentialSource, Secret};
use sim_kernel::{CapabilityName, Cx, Error, Ref, Result};
use std::{collections::BTreeMap, sync::Arc};

const SECRET_CAPABILITY: &str = "ai-runner-secret";

/// Preopened authority capable of resolving one provider credential.
///
/// Concrete sourcing remains private to the capsule implementing this trait.
pub trait SecretProvider: Send + Sync {
    /// Resolves credential material once for an opening provider seat.
    fn resolve(&self, cx: &mut Cx) -> Result<Secret>;
}

/// Process-local bindings from opaque refs to preopened secret providers.
#[derive(Default)]
pub struct SecretProviderRegistry {
    providers: BTreeMap<Ref, Arc<dyn SecretProvider>>,
}

impl SecretProviderRegistry {
    /// Creates an empty set of preopened secret-provider bindings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binds an opaque ref to one preopened provider.
    pub fn bind(&mut self, reference: Ref, provider: Arc<dyn SecretProvider>) -> Result<()> {
        if self.providers.insert(reference, provider).is_some() {
            return Err(Error::Eval(
                "secret provider reference is already bound".to_owned(),
            ));
        }
        Ok(())
    }

    /// Resolves a portable credential source at provider-open time.
    pub fn resolve(&self, cx: &mut Cx, source: &CredentialSource) -> Result<Option<Secret>> {
        match source {
            CredentialSource::SecretProvider(reference) => {
                cx.require(&CapabilityName::new(SECRET_CAPABILITY))?;
                let provider = self.providers.get(reference).ok_or_else(|| {
                    Error::Eval("provider seat secret provider is unavailable".to_owned())
                })?;
                provider.resolve(cx).map(Some)
            }
            CredentialSource::BrokerOwned | CredentialSource::None => Ok(None),
        }
    }

    /// Revokes a preopened provider binding.
    pub fn revoke(&mut self, reference: &Ref) -> bool {
        self.providers.remove(reference).is_some()
    }
}
