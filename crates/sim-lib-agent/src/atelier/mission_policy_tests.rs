use super::{
    AuthorityLevel, CrewTopology, MissionAdmission, MissionPlan, MissionRequest, MissionRole,
    MissionVeto, Placement, ToolCeiling,
};
use sim_codec::{Input, decode_with_codec, encode_with_codec};
use sim_codec_json::JsonCodecLib;
use sim_codec_lisp::LispCodecLib;
use sim_kernel::{Cx, DefaultFactory, EagerPolicy, EncodeOptions, ReadPolicy, Symbol};
use std::sync::Arc;

fn sym(value: &str) -> Symbol {
    Symbol::new(value)
}

fn frozen(topology: CrewTopology, sites: [&str; 3]) -> MissionPlan {
    MissionPlan::freeze(
        "sha256:agent-7-pack-closure",
        [
            MissionRole::new(sym("observer"), AuthorityLevel::Organize),
            MissionRole::new(sym("juror"), AuthorityLevel::ProposeLocal),
            MissionRole::new(sym("judge"), AuthorityLevel::RequestSensitive),
        ],
        [
            Placement::new(sym("observer"), sym(sites[0])),
            Placement::new(sym("juror"), sym(sites[1])),
            Placement::new(sym("judge"), sym(sites[2])),
        ],
        [ToolCeiling::new(sym("observer"), [sym("source/read")])],
        8,
        sym("bridge/human-gate"),
        false,
        topology,
        Some(sym("judge")),
        [sym("juror")],
    )
    .unwrap()
}

fn request(role: &str, authority: AuthorityLevel) -> MissionRequest {
    MissionRequest {
        role: sym(role),
        authority,
        tool: None,
        pack_closure: "sha256:agent-7-pack-closure".into(),
        selected_judge: None,
        hidden_adjacent_room_read: false,
        deterministic_verification: true,
        source_checks: true,
        capability_granted: true,
        privacy_fence_clear: true,
        budget_requested: 1,
    }
}

#[test]
fn every_authority_level_has_a_deterministic_fake_model_specimen() {
    let plan = frozen(CrewTopology::Solo, ["fake/a", "fake/b", "fake/c"]);
    let cases = [
        (
            "observer",
            AuthorityLevel::Observe,
            MissionAdmission::AdmitLocal,
        ),
        (
            "observer",
            AuthorityLevel::Organize,
            MissionAdmission::AdmitLocal,
        ),
        (
            "juror",
            AuthorityLevel::ProposeLocal,
            MissionAdmission::AdmitLocal,
        ),
        (
            "judge",
            AuthorityLevel::AdoptLocal,
            MissionAdmission::AdmitLocal,
        ),
        (
            "judge",
            AuthorityLevel::RequestExternal,
            MissionAdmission::Escalate { sensitive: false },
        ),
        (
            "judge",
            AuthorityLevel::RequestSensitive,
            MissionAdmission::Escalate { sensitive: true },
        ),
    ];
    for (role, level, expected) in cases {
        assert_eq!(plan.admit(&request(role, level)), expected);
    }
}

#[test]
fn model_placements_preserve_identical_authority_and_veto_outcomes() {
    let local = frozen(
        CrewTopology::SpeculateVerify,
        ["fake/local-small", "fake/local-large", "fake/local-judge"],
    );
    let market = frozen(
        CrewTopology::SpeculateVerify,
        ["fake/remote-a", "fake/remote-b", "fake/remote-c"],
    );
    let mut requests = vec![
        request("juror", AuthorityLevel::ProposeLocal),
        request("juror", AuthorityLevel::AdoptLocal),
    ];
    let mut privacy = request("observer", AuthorityLevel::Observe);
    privacy.privacy_fence_clear = false;
    requests.push(privacy);
    let mut budget = request("judge", AuthorityLevel::RequestExternal);
    budget.budget_requested = 9;
    requests.push(budget);
    for request in requests {
        assert_eq!(local.admit(&request), market.admit(&request));
    }
}

#[test]
fn all_topologies_are_replaceable_data_and_codec_round_trip() {
    let topologies = [
        CrewTopology::Solo,
        CrewTopology::SpeculateVerify,
        CrewTopology::ThreeTable,
        CrewTopology::JudgeJury,
        CrewTopology::PlacementMarket,
    ];
    let mut cx = Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(9),
    );
    let json = JsonCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&json).unwrap();
    let lisp = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
    for topology in topologies {
        let plan = frozen(topology, ["fake/a", "fake/b", "fake/c"]);
        for codec in [
            Symbol::qualified("codec", "json"),
            Symbol::qualified("codec", "lisp"),
        ] {
            let expr = plan.as_expr();
            let encoded =
                encode_with_codec(&mut cx, &codec, &expr, EncodeOptions::default()).unwrap();
            let input = match encoded {
                sim_codec::Output::Text(v) => Input::Text(v),
                sim_codec::Output::Bytes(v) => Input::Bytes(v),
            };
            assert!(
                decode_with_codec(&mut cx, &codec, input, ReadPolicy::default())
                    .unwrap()
                    .canonical_eq(&expr)
            );
        }
    }
}

#[test]
fn immutable_law_rejects_widening_judge_privacy_pack_and_mandatory_veto_failures() {
    let plan = frozen(CrewTopology::JudgeJury, ["fake/a", "fake/b", "fake/c"]);
    let mut adopt = request("judge", AuthorityLevel::AdoptLocal);
    adopt.selected_judge = Some(sym("judge"));
    assert_eq!(plan.admit(&adopt), MissionAdmission::AdmitLocal);

    let mut cases = Vec::new();
    cases.push((
        request("juror", AuthorityLevel::AdoptLocal),
        MissionVeto::SelfWideningRole,
    ));
    let mut self_judge = request("juror", AuthorityLevel::ProposeLocal);
    self_judge.selected_judge = Some(sym("juror"));
    self_judge.authority = AuthorityLevel::AdoptLocal;
    cases.push((self_judge, MissionVeto::SelfWideningRole));
    let mut selected_by_request = request("judge", AuthorityLevel::AdoptLocal);
    selected_by_request.selected_judge = Some(sym("juror"));
    cases.push((selected_by_request, MissionVeto::SelfSelectedJudge));
    let mut hidden = request("observer", AuthorityLevel::Observe);
    hidden.hidden_adjacent_room_read = true;
    cases.push((hidden, MissionVeto::HiddenAdjacentRoomRead));
    let mut pack = request("observer", AuthorityLevel::Observe);
    pack.pack_closure = "sha256:adjacent-pack".into();
    cases.push((pack, MissionVeto::PackClosureMismatch));
    let mut verify = request("observer", AuthorityLevel::Observe);
    verify.deterministic_verification = false;
    cases.push((verify, MissionVeto::DeterministicVerification));
    let mut source = request("observer", AuthorityLevel::Observe);
    source.source_checks = false;
    cases.push((source, MissionVeto::SourceCheck));
    let mut capability = request("observer", AuthorityLevel::Observe);
    capability.capability_granted = false;
    cases.push((capability, MissionVeto::CapabilityRefusal));
    let mut privacy = request("observer", AuthorityLevel::Observe);
    privacy.privacy_fence_clear = false;
    cases.push((privacy, MissionVeto::PrivacyFence));
    let mut budget = request("observer", AuthorityLevel::Observe);
    budget.budget_requested = 99;
    cases.push((budget, MissionVeto::BudgetExhausted));
    for (request, veto) in cases {
        assert_eq!(plan.admit(&request), MissionAdmission::Refuse(veto));
    }
}

#[test]
fn content_identity_binds_every_plan_input() {
    let first = frozen(CrewTopology::Solo, ["fake/a", "fake/b", "fake/c"]);
    let same = frozen(CrewTopology::Solo, ["fake/a", "fake/b", "fake/c"]);
    let moved = frozen(CrewTopology::Solo, ["fake/x", "fake/b", "fake/c"]);
    let topology = frozen(CrewTopology::ThreeTable, ["fake/a", "fake/b", "fake/c"]);
    assert_eq!(first.content_id(), same.content_id());
    assert_ne!(first.content_id(), moved.content_id());
    assert_ne!(first.content_id(), topology.content_id());
}
