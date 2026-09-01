use sim_roadmap_refine::{RankComponent, RankRelation, WorkProfile, compare_profiles};

fn main() {
    let parent = WorkProfile {
        mutable_owners: 2,
        ..WorkProfile::default()
    };
    let child = WorkProfile {
        mutable_owners: 1,
        ..WorkProfile::default()
    };
    match compare_profiles(&child, &parent) {
        RankRelation::Lower {
            first_difference: RankComponent::MutableOwners,
            child,
            parent,
        } => println!("strict descent at mutable owners: {child} < {parent}"),
        relation => panic!("expected strict mutable-owner descent, got {relation:?}"),
    }
}
