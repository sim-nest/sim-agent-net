use sim_kernel::Ref;

/// Portable description of who owns a provider seat's credential.
///
/// The mechanics used by a secret provider (environment, file, keyring,
/// interaction, or a remote service) deliberately remain behind its opaque
/// reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CredentialSource {
    /// Resolve credential material through a preopened secret provider.
    SecretProvider(Ref),
    /// Authentication is owned by the provider's broker or harness.
    BrokerOwned,
    /// The seat does not use credential material.
    None,
}
