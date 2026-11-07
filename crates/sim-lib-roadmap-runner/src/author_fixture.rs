// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;

use sim_roadmap_core::{
    AcceptanceContract, CapabilityEnvelope, ChangeEnvelope, Charter, CheckpointId, CheckpointSpec,
    EffectEnvelope, ImplementationGuide, Limits, OwnerEnvelope, PhaseBody, PhaseId, PhaseOrigin,
    PhaseSpec, ProofPolicy, ResourceEnvelope, RoadmapId, RoadmapSpec, SchemaId,
};
use sim_source_deck::{
    ByteContentId, DeckInput, DeckLimits, FragmentDecoder, IndexFragment, RepositorySnapshot,
    SourceDeck, SourceFile,
};

use super::{Family, FamilyMember, Stance, parse_member};

pub(super) fn spec() -> RoadmapSpec {
    spec_checkpoint("semantic-tests")
}

pub(super) fn spec_checkpoint(checkpoint: &str) -> RoadmapSpec {
    let root_id = PhaseId::new("root").unwrap();
    let leaf_id = PhaseId::new("values").unwrap();
    let leaf = PhaseSpec {
        id: leaf_id.clone(),
        parent: Some(root_id.clone()),
        title: "Define values".into(),
        intent: "State one checkpoint".into(),
        body: PhaseBody::Leaf {
            checkpoints: vec![CheckpointSpec {
                id: CheckpointId::new(checkpoint).unwrap(),
                statement: "Canonical laws pass".into(),
            }],
        },
        dependencies: vec![],
        owners: OwnerEnvelope::default(),
        resources: ResourceEnvelope::default(),
        effects: EffectEnvelope::default(),
        capabilities: CapabilityEnvelope::default(),
        changes: ChangeEnvelope::default(),
        acceptance: AcceptanceContract {
            policy: ProofPolicy::All,
            statements: BTreeMap::new(),
        },
        coverage: vec![],
        outputs: BTreeMap::new(),
        guide: ImplementationGuide::default(),
        origin: PhaseOrigin::Authored,
    };
    let root = PhaseSpec {
        id: root_id.clone(),
        parent: None,
        title: "Value roadmap".into(),
        intent: "State bounded authored intent".into(),
        body: PhaseBody::Composite {
            children: vec![leaf_id.clone()],
        },
        dependencies: vec![],
        owners: OwnerEnvelope::default(),
        resources: ResourceEnvelope::default(),
        effects: EffectEnvelope::default(),
        capabilities: CapabilityEnvelope::default(),
        changes: ChangeEnvelope::default(),
        acceptance: AcceptanceContract {
            policy: ProofPolicy::All,
            statements: BTreeMap::new(),
        },
        coverage: vec![],
        outputs: BTreeMap::new(),
        guide: ImplementationGuide::default(),
        origin: PhaseOrigin::Authored,
    };
    RoadmapSpec {
        schema: SchemaId::new("roadmap-v1").unwrap(),
        id: RoadmapId::new("value-roadmap").unwrap(),
        charter: Charter {
            title: "Values".into(),
            intent: "One roadmap states the checkpoint".into(),
        },
        root: root_id,
        phases: [(root.id.clone(), root), (leaf.id.clone(), leaf)]
            .into_iter()
            .collect(),
        imports: BTreeMap::new(),
        limits: Limits::DEFAULT,
    }
}

struct SilentDecoder;

impl FragmentDecoder for SilentDecoder {
    fn decode(&self, _: &[u8]) -> Result<IndexFragment, sim_source_deck::Failure> {
        Err(sim_source_deck::Failure::Decode("no fragment".into()))
    }
}

pub(super) fn deck(files: &[(&str, &[u8])]) -> SourceDeck {
    let decoder = SilentDecoder;
    let built: Vec<SourceFile> = files
        .iter()
        .map(|(path, bytes)| SourceFile {
            owner: "sim-agent-net".into(),
            path: (*path).into(),
            content_id: ByteContentId::of(bytes).unwrap(),
            bytes: bytes.to_vec(),
        })
        .collect();
    let total = built.iter().map(|file| file.bytes.len()).sum();
    sim_source_deck::build(DeckInput {
        repositories: vec![RepositorySnapshot {
            owner: "sim-agent-net".into(),
            repository: "sim-agent-net".into(),
            revision: "fixture".into(),
        }],
        fragments: vec![],
        certificates: vec![],
        files: built,
        excerpts: vec![],
        specimens: vec![],
        queries: vec![],
        limitations: vec![],
        limits: DeckLimits::strict(1, 0, files.len(), 0, 0, 0, total),
        decoder: &decoder,
    })
    .unwrap()
}

pub(super) struct Draft<'a> {
    name: &'a str,
    round: &'a str,
    stance: &'a str,
    path: &'a str,
    roadmap: &'a str,
    owner: &'a str,
    fact: Option<&'a str>,
    decision: &'a str,
    consult: Option<&'a str>,
}

impl<'a> Draft<'a> {
    pub(super) fn new(name: &'a str, round: &'a str, stance: &'a str, path: &'a str) -> Self {
        Self {
            name,
            round,
            stance,
            path,
            roadmap: "value-roadmap",
            owner: "sim-agent-net",
            fact: Some("semantic-tests"),
            decision: "select this source",
            consult: None,
        }
    }

    pub(super) fn owner(mut self, owner: &'a str) -> Self {
        self.owner = owner;
        self
    }

    pub(super) fn roadmap(mut self, roadmap: &'a str) -> Self {
        self.roadmap = roadmap;
        self
    }

    pub(super) fn fact(mut self, fact: Option<&'a str>) -> Self {
        self.fact = fact;
        self
    }

    pub(super) fn decision(mut self, decision: &'a str) -> Self {
        self.decision = decision;
        self
    }

    pub(super) fn consult(mut self, consult: Option<&'a str>) -> Self {
        self.consult = consult;
        self
    }

    pub(super) fn bytes(self) -> Vec<u8> {
        let mut text = format!(
            "family/v1\nname {name}\nround {round}\nstance {stance}\npath {path}\nargument the argument\nwork the work\ngate the gate\nratchet ratchet-{round}\nowner {owner}\nchecker checker-{round}\nroadmap {roadmap}\n",
            name = self.name,
            round = self.round,
            stance = self.stance,
            path = self.path,
            owner = self.owner,
            roadmap = self.roadmap,
        );
        if let Some(fact) = self.fact {
            text.push_str(&format!("fact {fact}\n"));
        }
        text.push_str("advice keep the source\n");
        text.push_str(&format!("decision {decision}\n", decision = self.decision));
        if let Some(consult) = self.consult {
            text.push_str(&format!("consult {consult}\n"));
        }
        text.into_bytes()
    }
}

pub(super) fn without(section: &str) -> Vec<u8> {
    let raw = Draft::new("chosen", "2", "selected", "family/chosen.txt").bytes();
    let text = String::from_utf8(raw).unwrap();
    let mut out = String::new();
    for line in text.lines() {
        if line.split_once(' ').is_some_and(|(key, _)| key == section) {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.into_bytes()
}

pub(super) fn bare(path: &str, stance: Stance, round: &str) -> FamilyMember {
    FamilyMember {
        source: b"not-parsed".to_vec(),
        name: round.to_owned(),
        round: round.to_owned(),
        stance,
        path: path.to_owned(),
        argument: "the argument".to_owned(),
        work: "the work".to_owned(),
        gate: "the gate".to_owned(),
        ratchet: format!("ratchet-{round}"),
        owner: "sim-agent-net".to_owned(),
        checker: format!("checker-{round}"),
        roadmap: "value-roadmap".to_owned(),
        facts: vec!["semantic-tests".to_owned()],
        advice: Vec::new(),
        decisions: vec!["select this source".to_owned()],
        consults: Vec::new(),
    }
}

pub(super) fn assemble(parts: &[Vec<u8>]) -> Family {
    Family::from_members(
        parts
            .iter()
            .map(|bytes| parse_member(bytes).unwrap())
            .collect(),
    )
    .unwrap()
}

pub(super) fn happy() -> (Family, SourceDeck, Vec<u8>, Vec<u8>) {
    let consulted = Draft::new("earlier", "1", "consulted", "family/consulted.txt")
        .decision("note the earlier source")
        .bytes();
    let chosen = Draft::new("chosen", "2", "selected", "family/chosen.txt")
        .consult(Some("1"))
        .bytes();
    let family = assemble(&[consulted.clone(), chosen.clone()]);
    let deck = deck(&[
        ("family/consulted.txt", &consulted),
        ("family/chosen.txt", &chosen),
    ]);
    (family, deck, consulted, chosen)
}
