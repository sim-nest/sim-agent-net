use super::*;

pub(super) fn validate_metadata(package: &TopologyPackage) -> Result<()> {
    match metadata(package, "profile") {
        Some(Expr::Symbol(value)) if value.to_string() == AGENT_CONDUCT_PROFILE => Ok(()),
        _ => Err(fail(format!(
            "metadata profile must be {AGENT_CONDUCT_PROFILE}"
        ))),
    }
}

pub(super) fn validate_boundary(package: &TopologyPackage) -> Result<()> {
    let inputs_ok = package
        .graph
        .nodes
        .iter()
        .filter(|node| node.verb.name.as_ref() == "in")
        .all(|node| node.output.as_ref() == Some(&run_frame_shape()));
    let outputs_ok = package
        .graph
        .nodes
        .iter()
        .filter(|node| node.verb.name.as_ref() == "out")
        .all(|node| node.input.as_ref() == Some(&completed_or_stopped_shape()));
    if !inputs_ok {
        return Err(fail("public input Shape must be agent/RunFrame"));
    }
    if !outputs_ok {
        return Err(fail(
            "public output Shape must be completed-or-stopped AgentRunFrame",
        ));
    }
    Ok(())
}

pub(super) fn card_index(cards: &[AgentStepCard]) -> Result<BTreeMap<Symbol, AgentStepCard>> {
    let mut out = BTreeMap::new();
    for card in cards {
        if out.insert(card.step_id.clone(), card.clone()).is_some() {
            return Err(fail(format!("duplicate AgentStepCard {}", card.step_id)));
        }
    }
    Ok(out)
}

pub(super) fn validate_calls_and_routes(
    package: &TopologyPackage,
    cards: &BTreeMap<Symbol, AgentStepCard>,
) -> Result<Vec<AgentStepCard>> {
    let mut selected = Vec::new();
    for node in package
        .graph
        .nodes
        .iter()
        .filter(|node| node.verb.name.as_ref() == "call")
    {
        let target = target_symbol(node).ok_or_else(|| {
            fail(format!(
                "call node {} target must be a step symbol",
                node.id.as_symbol()
            ))
        })?;
        let card = cards.get(target).ok_or_else(|| {
            fail(format!(
                "call target {target} has no registered AgentStepCard"
            ))
        })?;
        if card.input_shape != run_frame_shape() || card.output_shape != run_frame_shape() {
            return Err(fail(format!(
                "step Card {target} is not RunFrame-compatible"
            )));
        }
        let outgoing: Vec<_> = package
            .graph
            .edges
            .iter()
            .filter(|edge| edge.from.node == node.id)
            .collect();
        for outcome in &card.outcomes {
            if is_terminal(target) {
                break;
            }
            let predicate = outcome_predicate(outcome);
            let count = outgoing
                .iter()
                .filter(|edge| edge.when.as_ref() == Some(&predicate))
                .count();
            if count != 1 {
                return Err(fail(format!(
                    "call target {target} outcome {outcome} requires exactly one outgoing predicate route, found {count}"
                )));
            }
        }
        selected.push(card.clone());
    }
    Ok(selected)
}

pub(super) fn validate_terminal_admission(package: &TopologyPackage) -> Result<()> {
    let nodes: BTreeMap<_, _> = package
        .graph
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node))
        .collect();
    for out in package
        .graph
        .nodes
        .iter()
        .filter(|node| node.verb.name.as_ref() == "out")
    {
        for edge in package
            .graph
            .edges
            .iter()
            .filter(|edge| edge.to.node == out.id)
        {
            let source = nodes
                .get(&edge.from.node)
                .ok_or_else(|| fail("route target is not reachable"))?;
            if !target_symbol(source).is_some_and(is_terminal) {
                return Err(fail(format!(
                    "public output {} is not admitted by finish or stop",
                    out.id.as_symbol()
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn derive_roles(
    package: &TopologyPackage,
    cards: &[AgentStepCard],
) -> Result<Vec<Symbol>> {
    let mut roles = BTreeSet::new();
    for node in package
        .graph
        .nodes
        .iter()
        .filter(|node| node.verb.name.as_ref() == "call")
    {
        if let Some(role) = &node.role {
            roles.insert(role.clone());
        }
    }
    for card in cards {
        roles.extend(card.roles.iter().cloned());
    }
    Ok(roles.into_iter().collect())
}

pub(super) fn validate_declared_roles(package: &TopologyPackage, roles: &[Symbol]) -> Result<()> {
    let Some(value) = metadata(package, "requires-roles") else {
        return Ok(());
    };
    let Expr::List(items) = value else {
        return Err(fail("requires-roles metadata must be a symbol list"));
    };
    let mut declared = BTreeSet::new();
    for item in items {
        let Expr::Symbol(role) = item else {
            return Err(fail("requires-roles metadata must contain symbols"));
        };
        declared.insert(role.clone());
    }
    if declared != roles.iter().cloned().collect() {
        return Err(fail(
            "requires-roles metadata disagrees with roles derived from nodes and Cards",
        ));
    }
    Ok(())
}

pub(super) fn derive_capabilities(
    package: &TopologyPackage,
    cards: &[AgentStepCard],
) -> Vec<CapabilityName> {
    let mut out: BTreeSet<_> = package.capabilities.iter().cloned().collect();
    for card in cards {
        out.extend(card.capabilities.iter().cloned());
    }
    out.into_iter().collect()
}

pub(super) fn metadata<'a>(package: &'a TopologyPackage, name: &str) -> Option<&'a Expr> {
    let normalized = name.replace('-', "_");
    package
        .metadata
        .iter()
        .find(|(key, _)| key.name.as_ref() == normalized)
        .map(|(_, value)| value)
}

pub(super) fn target_symbol(node: &sim_lib_topology::Node) -> Option<&Symbol> {
    match &node.target {
        Some(Expr::Symbol(value)) => Some(value),
        _ => None,
    }
}

pub(super) fn is_terminal(target: &Symbol) -> bool {
    target.namespace.as_deref() == Some("agent.step")
        && matches!(target.name.as_ref(), "finish" | "stop")
}

fn outcome_predicate(outcome: &Symbol) -> Expr {
    Expr::Symbol(Symbol::qualified(
        "agent",
        format!("outcome-{}", outcome.name),
    ))
}

pub(super) fn card_expr(card: &AgentStepCard) -> Expr {
    Expr::Symbol(card.step_id.clone())
}

pub(super) fn fail(message: impl Into<String>) -> Error {
    Error::Eval(format!("agent conduct: {}", message.into()))
}
