#[cfg(test)]
mod tests {
    use sim_citizen::{CitizenRegistry, run_registry_conformance_expecting};
    use sim_kernel::{Expr, Symbol};
    use sim_lib_continuity::*;

    fn plan() -> ContinuityPlan {
        ContinuityPlan {
            schema_version: 1,
            plan_id: Symbol::qualified("continuity.plan", "hostile"),
            roles: vec![RoleDemand {
                role: Symbol::new("root"),
                root: true,
                required_services: vec![Symbol::new("voice")],
                fallbacks: vec![],
            }],
            available_services: vec![Symbol::new("voice")],
            max_freshness: 3,
            retention_turns: 16,
            disclosure: vec![],
            network: NetworkPolicy::Offline,
            allowed_network_routes: vec![],
        }
    }

    fn event(id: &str, sequence: u64, kind: &str) -> ContinuityEvent {
        ContinuityEvent {
            event_id: Symbol::qualified("event", id),
            sequence,
            logical_time: sequence,
            kind: Symbol::new(kind),
            role: Symbol::new("root"),
            lease: None,
            payload: Expr::Nil,
            disclosure: None,
        }
    }

    #[test]
    fn public_recipe_can_rebuild_an_empty_cache() {
        let plan = ContinuityPlan::default();
        let journal = MemoryJournal::default();
        assert_eq!(
            rebuild(&plan, journal.turns()).unwrap(),
            ContinuityState::default()
        );
        assert_eq!(CURRENT_SCHEMA_VERSION, 1);
    }

    #[test]
    fn every_public_value_has_a_shape_and_general_codec_round_trip() {
        let mut registry = CitizenRegistry::new();
        register_citizens(&mut registry).unwrap();
        run_registry_conformance_expecting(
            &mut sim_kernel::testing::bare_cx(),
            &registry,
            &[
                "continuity/Plan",
                "continuity/RoleDemand",
                "continuity/RouteLease",
                "continuity/Turn",
                "continuity/Event",
                "continuity/Intent",
                "continuity/Refusal",
            ],
        )
        .unwrap();
    }

    #[test]
    fn hostile_trace_replays_with_identical_turns_and_intents() {
        let plan = plan();
        let mut journal = MemoryJournal::default();
        let mut state = ContinuityState::default();
        state = journal
            .accept(&plan, &state, event("first", 0, "observed"))
            .unwrap();
        assert_eq!(
            journal
                .accept(&plan, &state, event("first", 0, "observed"))
                .unwrap(),
            state
        );
        assert!(matches!(
            journal.accept(&plan, &state, event("future", 3, "observed")),
            Err(JournalError::Refused(_))
        ));
        state = journal
            .accept(&plan, &state, event("cancel", 1, "cancel"))
            .unwrap();
        assert!(matches!(
            journal.accept(&plan, &state, event("late", 2, "observed")),
            Err(JournalError::Refused(_))
        ));
        let rebuilt = rebuild(&plan, journal.turns()).unwrap();
        assert_eq!(rebuilt, state);
        assert_eq!(rebuilt.turns, journal.turns());
    }

    #[test]
    fn policy_boundaries_fail_closed_without_creating_route_authority() {
        let mut plan = plan();
        let mut candidate = event("candidate", 0, "candidate");
        candidate.lease = Some(RouteLease {
            route: Symbol::new("remote"),
            observed_at: 0,
            expires_at: 2,
            services: vec![Symbol::new("voice")],
            networked: true,
        });
        let refusal = apply(&plan, &ContinuityState::default(), candidate.clone()).unwrap_err();
        assert_eq!(refusal.code, Symbol::new("network-policy"));

        plan.network = NetworkPolicy::AllowListed;
        plan.allowed_network_routes = vec![Symbol::new("remote")];
        let accepted = apply(&plan, &ContinuityState::default(), candidate).unwrap();
        assert_eq!(
            accepted.turns[0].intents[0].route,
            Some(Symbol::new("remote"))
        );

        let mut disclosed = event("secret", 0, "observed");
        disclosed.disclosure = Some(Symbol::new("private"));
        assert_eq!(
            apply(&plan, &ContinuityState::default(), disclosed)
                .unwrap_err()
                .code,
            Symbol::new("disclosure-policy")
        );

        let mut invalid = plan;
        invalid.roles.push(invalid.roles[0].clone());
        assert!(invalid.validate().is_err());
        assert_eq!(migrate(1, Expr::Nil).unwrap(), Expr::Nil);
        assert!(matches!(
            migrate(0, Expr::Nil),
            Err(MigrationError::UnsupportedVersion(0))
        ));
    }

    #[test]
    fn bounded_cache_rebuilds_from_the_complete_journal() {
        let mut plan = plan();
        plan.retention_turns = 1;
        let mut journal = MemoryJournal::default();
        let mut state = ContinuityState::default();
        state = journal
            .accept(&plan, &state, event("zero", 0, "observed"))
            .unwrap();
        state = journal
            .accept(&plan, &state, event("one", 1, "observed"))
            .unwrap();
        assert_eq!(state.turns.len(), 1);
        assert_eq!(journal.turns().len(), 2);
        assert_eq!(rebuild(&plan, journal.turns()).unwrap(), state);
    }
}
