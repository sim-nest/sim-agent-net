//! Capability names used by remote-backed tables.

use sim_kernel::CapabilityName;

/// The capability gating remote tables (`table.remote`).
pub fn table_remote_capability() -> CapabilityName {
    CapabilityName::new("table.remote")
}

#[cfg(test)]
mod tests {
    use super::table_remote_capability;

    #[test]
    fn capability_token_is_stable() {
        assert_eq!(table_remote_capability().as_str(), "table.remote");
    }
}
