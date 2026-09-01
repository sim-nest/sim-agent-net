use crate::WorkProfile;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TractabilityPolicy {
    pub revision: String,
    pub maximum: WorkProfile,
    pub maximum_children: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyBreach {
    pub component: crate::RankComponent,
    pub actual: u32,
    pub maximum: u32,
}

impl TractabilityPolicy {
    pub fn breaches(&self, profile: &WorkProfile) -> Vec<PolicyBreach> {
        crate::ORDINAL_COMPONENTS
            .into_iter()
            .filter_map(|component| {
                let actual = component_value(profile, component);
                let maximum = component_value(&self.maximum, component);
                (actual > maximum).then_some(PolicyBreach {
                    component,
                    actual,
                    maximum,
                })
            })
            .collect()
    }
}

fn component_value(profile: &WorkProfile, component: crate::RankComponent) -> u32 {
    match component {
        crate::RankComponent::Unknowns => profile.unknowns,
        crate::RankComponent::MutableOwners => profile.mutable_owners,
        crate::RankComponent::Packages => profile.packages,
        crate::RankComponent::ChangeTargets => profile.change_targets,
        crate::RankComponent::Promises => profile.promises,
        crate::RankComponent::AcceptanceGroups => profile.acceptance_groups,
        crate::RankComponent::Checkpoints => profile.checkpoints,
    }
}
