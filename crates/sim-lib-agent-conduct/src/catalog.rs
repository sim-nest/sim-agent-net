/// Result of one domain step driven by a bounded conduct edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundedEdgeStep<T> {
    /// The domain operation reached its public result.
    Complete(T),
    /// Route the value across the catalog edge and run the next domain step.
    Continue(T),
}

/// Runs domain work under one checked, bounded edge from a shipped conduct.
///
/// Compatibility APIs use this adapter when their historical payload is not an
/// `AgentRunFrame`. Repetition and its visit accounting still belong here, next
/// to the certified topology package; the adapter closure performs exactly one
/// model, tool, review, or revision step.
pub fn run_catalog_bounded_edge<T, F>(
    catalog_id: &str,
    from: &str,
    to: &str,
    max_visits: u32,
    initial: T,
    mut step: F,
) -> Result<T>
where
    F: FnMut(T, u32) -> Result<BoundedEdgeStep<T>>,
{
    let source = CATALOG
        .iter()
        .find(|source| source.id == catalog_id)
        .ok_or_else(|| fail(format!("unknown catalog conduct {catalog_id}")))?;
    let package = parse_package(source.source)?;
    let edge = package
        .graph
        .edges
        .iter()
        .find(|edge| {
            edge.from.node.as_symbol().name.as_ref() == from
                && edge.to.node.as_symbol().name.as_ref() == to
        })
        .ok_or_else(|| fail(format!("{catalog_id} has no {from} -> {to} edge")))?;
    // The package proves that this is a bounded cyclic route. Compatibility
    // policies may tighten or extend its domain bound while the scheduler keeps
    // ownership of visit accounting.
    let _catalog_cap = edge
        .max_visits
        .unwrap_or(package.graph.budget.max_edge_visits);
    let limit = max_visits;
    let mut value = initial;
    for visit in 0..=limit {
        match step(value, visit)? {
            BoundedEdgeStep::Complete(done) => return Ok(done),
            BoundedEdgeStep::Continue(next) if visit < limit => value = next,
            BoundedEdgeStep::Continue(_) => {
                return Err(fail(format!(
                    "{catalog_id} edge {from} -> {to} exhausted after {limit} visit(s)"
                )));
            }
        }
    }
    unreachable!("bounded catalog edge always completes or exhausts")
}

/// One immutable package shipped in the standard agent-kind catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentConductCatalogSource {
    /// Public package identity and graph name.
    pub id: &'static str,
    /// Complete `.simtopo` package text.
    pub source: &'static str,
}

/// A catalog package loaded through the topology registry and certified as a conduct.
#[derive(Clone, Debug)]
pub struct AgentConductCatalogEntry {
    /// Public package identity.
    pub id: Symbol,
    /// Certified conduct projection.
    pub conduct: AgentConduct,
    /// Number of deterministic tests embedded in the package.
    pub embedded_tests: usize,
}

const CATALOG: &[AgentConductCatalogSource] = &[
    AgentConductCatalogSource {
        id: "agent/default-v1",
        source: include_str!("../catalog/default-v1.simtopo"),
    },
    AgentConductCatalogSource {
        id: "agent/react-v1",
        source: include_str!("../catalog/react-v1.simtopo"),
    },
    AgentConductCatalogSource {
        id: "agent/plan-act-replan-v1",
        source: include_str!("../catalog/plan-act-replan-v1.simtopo"),
    },
    AgentConductCatalogSource {
        id: "agent/phased-v1",
        source: include_str!("../catalog/phased-v1.simtopo"),
    },
    AgentConductCatalogSource {
        id: "agent/verify-retry-v1",
        source: include_str!("../catalog/verify-retry-v1.simtopo"),
    },
    AgentConductCatalogSource {
        id: "agent/router-crew-v1",
        source: include_str!("../catalog/router-crew-v1.simtopo"),
    },
    AgentConductCatalogSource {
        id: "agent/triage-v1",
        source: include_str!("../catalog/triage-v1.simtopo"),
    },
];

/// Returns the shipped data sources in stable public-id order.
pub fn agent_conduct_catalog_sources() -> &'static [AgentConductCatalogSource] {
    CATALOG
}

/// Loads every catalog package through a table-backed topology source and registry, then
/// certifies the `agent/conduct-v1` profile against the supplied Card catalog.
pub fn load_agent_conduct_catalog(
    cx: &mut Cx,
    registry: &mut TopologyRegistry,
    step_cards: &[AgentStepCard],
) -> Result<Vec<AgentConductCatalogEntry>> {
    let rows = CATALOG
        .iter()
        .map(|item| {
            Ok((
                Symbol::new(item.id),
                cx.factory().string(item.source.to_owned())?,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let table = cx.new_table(rows)?;
    CATALOG
        .iter()
        .map(|item| {
            let key = Symbol::new(item.id);
            registry.load_source(
                cx,
                TopologyPackageSource::table_entry(table.clone(), key.clone()),
            )?;
            let package = parse_package(item.source)?;
            let embedded_tests = package.tests.len();
            let conduct = validate_agent_conduct(package, step_cards)?;
            Ok(AgentConductCatalogEntry {
                id: key,
                conduct,
                embedded_tests,
            })
        })
        .collect()
}
