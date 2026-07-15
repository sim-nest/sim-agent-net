use sim_kernel::{CapabilityName, CapabilitySet, Result, Symbol, diminish};

/// Resolves capability symbols and intersects them with the active set.
pub fn effective_ceiling(current: &CapabilitySet, names: &[Symbol]) -> Result<CapabilitySet> {
    let allowed = names
        .iter()
        .map(capability_name)
        .fold(CapabilitySet::new(), CapabilitySet::grant);
    Ok(diminish(current, &allowed))
}

fn capability_name(symbol: &Symbol) -> CapabilityName {
    match symbol.namespace.as_deref() {
        Some("capability") => CapabilityName::new(symbol.name.to_string()),
        _ => CapabilityName::new(symbol.as_qualified_str()),
    }
}

#[cfg(test)]
mod tests {
    use super::effective_ceiling;
    use sim_kernel::{CapabilityName, CapabilitySet, Symbol};

    #[test]
    fn effective_ceiling_never_grows() {
        let ai_run = CapabilityName::new("ai/run");
        let fs_read = CapabilityName::new("fs/read");
        let current = CapabilitySet::new()
            .grant(ai_run.clone())
            .grant(fs_read.clone());

        let narrowed = effective_ceiling(
            &current,
            &[
                Symbol::qualified("ai", "run"),
                Symbol::qualified("net", "write"),
            ],
        )
        .unwrap();

        assert!(narrowed.contains(&ai_run));
        assert!(!narrowed.contains(&fs_read));
        assert!(!narrowed.contains(&CapabilityName::new("net/write")));
    }

    #[test]
    fn capability_namespace_resolves_to_bare_name() {
        let read_eval = CapabilityName::new("read-eval");
        let current = CapabilitySet::new().grant(read_eval.clone());

        let narrowed =
            effective_ceiling(&current, &[Symbol::qualified("capability", "read-eval")]).unwrap();

        assert!(narrowed.contains(&read_eval));
    }
}
