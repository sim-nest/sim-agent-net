use crate::{AuthMethod, SessionStatus};

/// Machine-readable broker compatibility declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrokerRevision {
    /// Opaque executable path identity selected by the host.
    pub executable_path: String,
    /// Exact supported broker version.
    pub version: String,
    /// Machine-readable mode name; interactive modes are never accepted.
    pub machine_mode: String,
    /// Authentication methods structurally supported by this revision.
    pub auth_methods: Vec<AuthMethod>,
    /// Exact event schema revision.
    pub event_schema: String,
}

/// Typed result of a provider control operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderControlResult {
    /// Supported authentication methods.
    AuthMethods(Vec<AuthMethod>),
    /// Current or newly established session status.
    Session(SessionStatus),
    /// A logout completed and no session remains.
    LoggedOut,
}

/// Provider control operation names, deliberately separate from inference.
pub mod operation {
    use sim_kernel::Symbol;

    /// `provider/auth-methods`.
    pub fn auth_methods() -> Symbol {
        Symbol::qualified("provider", "auth-methods")
    }
    /// `provider/login`.
    pub fn login() -> Symbol {
        Symbol::qualified("provider", "login")
    }
    /// `provider/status`.
    pub fn status() -> Symbol {
        Symbol::qualified("provider", "status")
    }
    /// `provider/logout`.
    pub fn logout() -> Symbol {
        Symbol::qualified("provider", "logout")
    }

    /// All standard provider control operations.
    pub fn all() -> Vec<Symbol> {
        vec![auth_methods(), login(), status(), logout()]
    }
}
