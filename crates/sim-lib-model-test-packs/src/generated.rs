use crate::PackError;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum GeneratedFamily {
    SymbolicTree,
    StateTrace,
    ConstraintPlan,
    CausalDebug,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenerationBounds {
    pub max_depth: u16,
    pub max_entities: u16,
    pub max_operations: u16,
    pub max_prompt_bytes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedTask {
    pub revision: String,
    pub family: GeneratedFamily,
    pub seed: u64,
    pub pair_id: String,
    pub is_control: bool,
    pub facts: Vec<String>,
    pub operations: Vec<String>,
    pub entities: Vec<String>,
    pub dependency_wiring: Vec<(u16, u16)>,
    pub answer_schema: String,
    pub renderer: String,
    pub prompt_bytes: u32,
    pub expected_answer: String,
    pub bounds: GenerationBounds,
}

impl GeneratedTask {
    pub fn verify(&self, answer: &str) -> bool {
        answer == self.expected_answer
    }
    pub fn validate(&self) -> Result<(), PackError> {
        if self.pair_id.is_empty()
            || self.answer_schema.is_empty()
            || self.renderer.is_empty()
            || self.prompt_bytes > self.bounds.max_prompt_bytes
            || self.entities.len() > usize::from(self.bounds.max_entities)
            || self.operations.len() > usize::from(self.bounds.max_operations)
            || self
                .dependency_wiring
                .iter()
                .any(|(a, b)| *a >= self.bounds.max_depth || *b >= self.bounds.max_depth)
        {
            return Err(PackError::Bounds);
        }
        let revision = revision_for(self);
        if revision != self.revision {
            return Err(PackError::Determinism);
        }
        Ok(())
    }
}

pub fn generate_pair(
    family: GeneratedFamily,
    seed: u64,
    bounds: GenerationBounds,
) -> Result<(GeneratedTask, GeneratedTask), PackError> {
    if bounds.max_depth < 3
        || bounds.max_entities < 3
        || bounds.max_operations < 2
        || bounds.max_prompt_bytes < 128
    {
        return Err(PackError::Bounds);
    }
    let facts = vec![
        format!("seed={seed}"),
        "alpha precedes beta".into(),
        "beta precedes gamma".into(),
    ];
    let operations = vec!["observe".into(), "derive".into()];
    let entities = vec!["alpha".into(), "beta".into(), "gamma".into()];
    let pair_id = format!("{:?}-{seed}", family).to_ascii_lowercase();
    let base = GeneratedTask {
        revision: String::new(),
        family,
        seed,
        pair_id,
        is_control: false,
        facts,
        operations,
        entities,
        dependency_wiring: vec![(0, 1), (1, 2)],
        answer_schema: "ordered-entity/v1".into(),
        renderer: "bounded-text/v1".into(),
        prompt_bytes: 128,
        expected_answer: "alpha,beta,gamma".into(),
        bounds,
    };
    let mut target = base.clone();
    target.revision = revision_for(&target);
    let mut control = base;
    control.is_control = true;
    control.dependency_wiring = vec![(0, 1), (0, 2)];
    control.expected_answer = "alpha,beta|gamma".into();
    control.revision = revision_for(&control);
    target.validate()?;
    control.validate()?;
    Ok((target, control))
}

fn revision_for(task: &GeneratedTask) -> String {
    let mut h = Sha256::new();
    h.update(format!(
        "{:?}|{}|{}|{:?}|{}",
        task.family, task.seed, task.is_control, task.dependency_wiring, task.expected_answer
    ));
    format!("sha256:{:x}", h.finalize())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TrialState {
    Pass,
    Fail,
    Censored,
}
#[derive(Clone, Debug, PartialEq)]
pub struct PairTrial {
    pub family: GeneratedFamily,
    pub pair_id: String,
    pub depth: u16,
    pub target: TrialState,
    pub control: TrialState,
}
#[derive(Clone, Debug, PartialEq)]
pub struct FamilyCapability {
    pub combinations: BTreeMap<(TrialState, TrialState), u32>,
    pub inversions: u32,
    pub censored: u32,
    pub isotonic_curve: Vec<(u16, f64)>,
    pub threshold: Option<u16>,
}

pub fn capability_by_family(
    trials: &[PairTrial],
    pass_threshold: f64,
) -> BTreeMap<GeneratedFamily, FamilyCapability> {
    let mut families = BTreeMap::new();
    for family in [
        GeneratedFamily::SymbolicTree,
        GeneratedFamily::StateTrace,
        GeneratedFamily::ConstraintPlan,
        GeneratedFamily::CausalDebug,
    ] {
        let rows: Vec<_> = trials.iter().filter(|x| x.family == family).collect();
        if rows.is_empty() {
            continue;
        }
        let mut combinations = BTreeMap::new();
        let mut censored = 0;
        let mut inversions = 0;
        let mut by_depth: BTreeMap<u16, (u32, u32)> = BTreeMap::new();
        let mut pair_ids = BTreeSet::new();
        for row in rows {
            if !pair_ids.insert(&row.pair_id) {
                continue;
            }
            *combinations.entry((row.target, row.control)).or_insert(0) += 1;
            if matches!(row.target, TrialState::Censored)
                | matches!(row.control, TrialState::Censored)
            {
                censored += 1;
                continue;
            }
            if row.target == TrialState::Pass && row.control == TrialState::Fail {
                inversions += 1
            }
            let e = by_depth.entry(row.depth).or_default();
            e.1 += 1;
            if row.target == TrialState::Pass {
                e.0 += 1
            }
        }
        let mut curve = Vec::new();
        let mut floor = 0.0_f64;
        for (depth, (pass, total)) in by_depth {
            let raw = f64::from(pass) / f64::from(total);
            floor = floor.max(raw);
            curve.push((depth, floor));
        }
        let threshold = curve
            .iter()
            .find(|(_, p)| *p >= pass_threshold)
            .map(|(d, _)| *d);
        families.insert(
            family,
            FamilyCapability {
                combinations,
                inversions,
                censored,
                isotonic_curve: curve,
                threshold,
            },
        );
    }
    families
}

pub const FORBIDDEN_LANGUAGE_PACK_PATTERNS: &[&str] = &[
    "replacement interpreter",
    "replacement parser",
    "replacement runtime",
    "emit source then parse",
    "source-text reparse",
];
pub fn guard_language_pack(text: &str) -> Result<(), PackError> {
    let lower = text.to_ascii_lowercase();
    if FORBIDDEN_LANGUAGE_PACK_PATTERNS
        .iter()
        .any(|p| lower.contains(p))
    {
        Err(PackError::Repository)
    } else {
        Ok(())
    }
}
