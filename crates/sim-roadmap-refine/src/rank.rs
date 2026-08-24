use crate::WorkProfile;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RankComponent {
    Unknowns,
    MutableOwners,
    Packages,
    ChangeTargets,
    Promises,
    AcceptanceGroups,
    Checkpoints,
}

pub const ORDINAL_COMPONENTS: [RankComponent; 7] = [
    RankComponent::Unknowns,
    RankComponent::MutableOwners,
    RankComponent::Packages,
    RankComponent::ChangeTargets,
    RankComponent::Promises,
    RankComponent::AcceptanceGroups,
    RankComponent::Checkpoints,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RankRelation {
    Equal,
    Lower {
        first_difference: RankComponent,
        child: u32,
        parent: u32,
    },
    Higher {
        first_difference: RankComponent,
        child: u32,
        parent: u32,
    },
}

pub fn compare_profiles(child: &WorkProfile, parent: &WorkProfile) -> RankRelation {
    for component in ORDINAL_COMPONENTS {
        let (child_value, parent_value) = (value(child, component), value(parent, component));
        if child_value < parent_value {
            return RankRelation::Lower {
                first_difference: component,
                child: child_value,
                parent: parent_value,
            };
        }
        if child_value > parent_value {
            return RankRelation::Higher {
                first_difference: component,
                child: child_value,
                parent: parent_value,
            };
        }
    }
    RankRelation::Equal
}

fn value(profile: &WorkProfile, component: RankComponent) -> u32 {
    match component {
        RankComponent::Unknowns => profile.unknowns,
        RankComponent::MutableOwners => profile.mutable_owners,
        RankComponent::Packages => profile.packages,
        RankComponent::ChangeTargets => profile.change_targets,
        RankComponent::Promises => profile.promises,
        RankComponent::AcceptanceGroups => profile.acceptance_groups,
        RankComponent::Checkpoints => profile.checkpoints,
    }
}
