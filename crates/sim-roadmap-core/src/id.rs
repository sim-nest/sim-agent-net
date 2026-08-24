use std::fmt;

use sim_kernel::ContentId;

use crate::{Failure, Limits};

macro_rules! text_id {
    ($name:ident) => {
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, Failure> {
                let value = value.into();
                validate_id(stringify!($name), &value, Limits::DEFAULT.id_bytes)?;
                Ok(Self(value))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

text_id!(RoadmapId);
text_id!(PhaseId);
text_id!(CheckpointId);
text_id!(ImportId);
text_id!(OutputId);
text_id!(ObligationId);
text_id!(PromiseId);
text_id!(SketchId);
text_id!(ProfileId);
text_id!(OwnerId);
text_id!(ResourceId);
text_id!(EffectId);
text_id!(CapabilityId);
text_id!(ChangeId);
text_id!(SchemaId);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoadmapRevisionId(pub ContentId);

fn validate_id(kind: &'static str, value: &str, maximum: usize) -> Result<(), Failure> {
    if value.is_empty() {
        return Err(Failure::InvalidText {
            kind,
            reason: "empty",
        });
    }
    if value.len() > maximum {
        return Err(Failure::OverLimit {
            limit: "id_bytes",
            actual: value.len(),
            maximum,
        });
    }
    if value.chars().any(char::is_control) {
        return Err(Failure::InvalidText {
            kind,
            reason: "control character",
        });
    }
    if value == "." || value == ".." || value.contains('/') || value.contains('\\') {
        return Err(Failure::InvalidText {
            kind,
            reason: "path-like",
        });
    }
    Ok(())
}
