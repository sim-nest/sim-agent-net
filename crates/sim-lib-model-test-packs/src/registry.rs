use crate::{FixtureEpoch, PackError, PackManifest, PackPrivacy, validate_epoch};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
pub struct PackRegistry {
    public: BTreeMap<(String, String), (PackManifest, FixtureEpoch)>,
}
impl PackRegistry {
    pub fn register_public(
        &mut self,
        manifest: PackManifest,
        epoch: FixtureEpoch,
    ) -> Result<(), PackError> {
        if manifest.privacy != PackPrivacy::Public {
            return Err(PackError::PrivateExport);
        }
        validate_epoch(&manifest, &epoch)?;
        if !manifest.hidden_grader_ids.is_empty() {
            return Err(PackError::PrivateExport);
        }
        let closure: BTreeSet<_> = manifest.closure.iter().map(|f| f.path.as_str()).collect();
        for test in &manifest.public_tests {
            if !closure.contains(test.as_str()) {
                return Err(PackError::TestFailure(test.clone()));
            }
        }
        let key = (manifest.id.clone(), manifest.revision.clone());
        if self.public.insert(key, (manifest, epoch)).is_some() {
            return Err(PackError::Duplicate("pack revision"));
        }
        Ok(())
    }
    pub fn get(&self, id: &str, revision: &str) -> Option<&(PackManifest, FixtureEpoch)> {
        self.public.get(&(id.into(), revision.into()))
    }
    pub fn len(&self) -> usize {
        self.public.len()
    }
    pub fn is_empty(&self) -> bool {
        self.public.is_empty()
    }
}
