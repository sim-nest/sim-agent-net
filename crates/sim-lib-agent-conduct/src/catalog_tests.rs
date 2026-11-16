// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::*;

fn catalog_cards() -> Vec<AgentStepCard> {
    let specs: &[(&str, &[&str], &[&str])] = &[
        ("component", &["component"], &["continue", "error"]),
        ("delegate", &["delegate"], &["continue", "error"]),
        ("finish", &[], &["finished", "error"]),
        ("model-turn", &["runner"], &["tool-calls", "final", "error"]),
        ("plan", &["runner"], &["created", "error"]),
        (
            "replan",
            &["runner"],
            &["keep", "replace", "done", "stop", "error"],
        ),
        (
            "review",
            &["reviewer"],
            &["accept", "revise", "reject", "error"],
        ),
        ("stop", &[], &["stopped"]),
        ("tool-batch", &["tools"], &["continue", "final", "error"]),
    ];
    specs
        .iter()
        .map(|(id, roles, outcomes)| AgentStepCard {
            step_id: Symbol::qualified("agent.step", *id),
            input_shape: run_frame_shape(),
            output_shape: run_frame_shape(),
            roles: roles.iter().map(|role| Symbol::new(*role)).collect(),
            outcomes: outcomes
                .iter()
                .map(|outcome| Symbol::new(*outcome))
                .collect(),
            ..Default::default()
        })
        .collect()
}

#[test]
fn data_catalog_loads_through_registry_and_normalizes_to_distinct_reports() {
    let mut cx = sim_kernel::testing::bare_cx();
    cx.grant(sim_lib_topology::topology_write_capability());
    let mut registry = TopologyRegistry::new();
    let entries = load_agent_conduct_catalog(&mut cx, &mut registry, &catalog_cards()).unwrap();
    assert_eq!(entries.len(), 7);
    assert_eq!(registry.list().len(), 7);
    assert!(entries.iter().all(|entry| entry.embedded_tests == 1));
    let reports = entries
        .iter()
        .map(|entry| {
            let graph = &entry.conduct.topology.graph;
            format!(
                "{}|{}|{}|{}|{:?}",
                entry.id,
                graph.nodes.len(),
                graph.edges.len(),
                graph.budget.max_steps,
                entry.conduct.required_roles
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(reports.len(), 7);
}

#[test]
fn every_catalog_package_has_embedded_contract_data() {
    assert_eq!(
        agent_conduct_catalog_sources().last().unwrap().id,
        "agent/triage-v1"
    );
    for item in agent_conduct_catalog_sources() {
        let package = parse_package(item.source).unwrap();
        for field in ["profile", "result_shape", "domain_budget", "diagram"] {
            assert!(
                package
                    .metadata
                    .iter()
                    .any(|(key, _)| key.name.as_ref() == field),
                "{} missing {field}",
                item.id
            );
        }
        assert_eq!(package.tests.len(), 1);
        assert!(package.graph.budget.max_steps > 0);
    }
    let implementation = include_str!("lib.rs");
    assert!(!implementation.contains("struct Triage"));
    assert!(!implementation.contains("fn triage"));
}

#[test]
fn catalog_bounded_edge_owns_visit_order_and_exhaustion() {
    let mut visits = Vec::new();
    let result =
        run_catalog_bounded_edge("agent/react-v1", "answer", "tools", 2, 0, |value, visit| {
            visits.push(visit);
            if visit == 2 {
                Ok(BoundedEdgeStep::Complete(value + 1))
            } else {
                Ok(BoundedEdgeStep::Continue(value + 1))
            }
        })
        .unwrap();
    assert_eq!(result, 3);
    assert_eq!(visits, vec![0, 1, 2]);

    let error =
        run_catalog_bounded_edge("agent/verify-retry-v1", "verify", "act", 1, (), |(), _| {
            Ok(BoundedEdgeStep::Continue(()))
        })
        .unwrap_err();
    assert!(error.to_string().contains("exhausted after 1 visit(s)"));
}
// conformance: conduct topology profile tests load the catalog, require embedded contract data, and exhaust bounded edges without a kind-specific loop.
