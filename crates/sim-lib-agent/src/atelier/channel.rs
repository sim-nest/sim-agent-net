//! Payload-free, authority-diminishing passports for cross-surface handoff.

use std::collections::BTreeSet;

/// A surface class with a fixed disclosure ceiling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    /// Full workstation review surface.
    Workstation,
    /// Local, ephemeral voice response.
    LocalVoice,
    /// Telegram text relay.
    Telegram,
    /// Mono-HUD Halo glance.
    Halo,
    /// Body-level watch glance.
    Watch,
}

/// Maximum response form admitted before a channel encoder is called.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum ChannelCeiling {
    /// No response is encoded.
    Silent,
    /// One acknowledgement or status token.
    Acknowledge,
    /// A short, non-sensitive summary.
    Summary,
    /// A bounded interactive answer.
    Interactive,
    /// A full review document.
    Full,
}

impl Channel {
    /// Returns the richest response this channel may encode.
    pub const fn ceiling(self) -> ChannelCeiling {
        match self {
            Self::Workstation => ChannelCeiling::Full,
            Self::LocalVoice => ChannelCeiling::Interactive,
            Self::Telegram => ChannelCeiling::Summary,
            Self::Halo | Self::Watch => ChannelCeiling::Acknowledge,
        }
    }

    /// Reduces a requested response to the channel ceiling.
    pub fn reduce(self, requested: ChannelCeiling) -> ChannelCeiling {
        requested.min(self.ceiling())
    }
}

/// The Shape the destination must satisfy when returning control.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReturnShape(pub String);

/// A deliberately payload-free handoff passport.
///
/// It contains references and diminished grant names only. Credentials, live
/// handles, model memory and message payloads have no representation here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChannelPassport {
    /// Expedition whose thought is moving between surfaces.
    pub expedition_id: String,
    /// Digest of the closed pack, never the pack itself.
    pub pack_closure_digest: String,
    /// Explicitly selected, non-secret claim references.
    pub claim_refs: BTreeSet<String>,
    /// Absolute expiry in the caller's monotonic time domain.
    pub expires_at: u64,
    /// Required return value Shape.
    pub return_shape: ReturnShape,
    /// Diminished grant identifiers.
    pub grants: BTreeSet<String>,
}

/// Refusal from a passport handoff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HandoffError {
    /// The source passport has expired; manual continuation may create a new one.
    Expired,
    /// The destination attempted to gain authority or disclosure.
    AuthorityGain,
    /// Expedition, closure, or return contract changed in transit.
    IdentityChanged,
}

impl ChannelPassport {
    /// Verifies a derived passport only preserves or removes authority and disclosure.
    pub fn verify_handoff(&self, next: &Self, now: u64) -> Result<(), HandoffError> {
        if now >= self.expires_at || now >= next.expires_at {
            return Err(HandoffError::Expired);
        }
        if self.expedition_id != next.expedition_id
            || self.pack_closure_digest != next.pack_closure_digest
            || self.return_shape != next.return_shape
        {
            return Err(HandoffError::IdentityChanged);
        }
        if next.expires_at > self.expires_at
            || !next.claim_refs.is_subset(&self.claim_refs)
            || !next.grants.is_subset(&self.grants)
        {
            return Err(HandoffError::AuthorityGain);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passport() -> ChannelPassport {
        ChannelPassport {
            expedition_id: "expedition-7".into(),
            pack_closure_digest: "sha256:closed".into(),
            claim_refs: ["claim:summary", "claim:location"].map(String::from).into(),
            expires_at: 100,
            return_shape: ReturnShape("bridge/return".into()),
            grants: ["reply", "read-summary"].map(String::from).into(),
        }
    }

    #[test]
    fn adversarial_handoffs_never_gain_grants_or_disclosure() {
        let source = passport();
        let mut diminished = source.clone();
        diminished.grants.remove("reply");
        diminished.claim_refs.remove("claim:location");
        diminished.expires_at = 90;
        assert_eq!(source.verify_handoff(&diminished, 10), Ok(()));

        let mut richer = diminished.clone();
        richer.grants.insert("credential-read".into());
        assert_eq!(source.verify_handoff(&richer, 10), Err(HandoffError::AuthorityGain));
        let mut disclosed = diminished;
        disclosed.claim_refs.insert("claim:model-memory".into());
        assert_eq!(source.verify_handoff(&disclosed, 10), Err(HandoffError::AuthorityGain));
    }

    #[test]
    fn expiry_and_channel_ceilings_fail_closed() {
        assert_eq!(passport().verify_handoff(&passport(), 100), Err(HandoffError::Expired));
        assert_eq!(Channel::Watch.reduce(ChannelCeiling::Full), ChannelCeiling::Acknowledge);
        assert_eq!(Channel::Halo.reduce(ChannelCeiling::Interactive), ChannelCeiling::Acknowledge);
        assert_eq!(Channel::Telegram.reduce(ChannelCeiling::Full), ChannelCeiling::Summary);
        assert_eq!(Channel::LocalVoice.reduce(ChannelCeiling::Full), ChannelCeiling::Interactive);
        assert_eq!(Channel::Workstation.reduce(ChannelCeiling::Full), ChannelCeiling::Full);
    }
}
