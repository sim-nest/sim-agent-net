use std::collections::{BTreeMap, BTreeSet};

use sim_kernel::{ContentId, Error, Result, Symbol};

use crate::{CompiledIntent, IntentStatus};

/// Named index for compiled intent artifacts.
///
/// The index stores `CompiledIntent` records, which point at normalized prose
/// and BRIDGE packet bytes by `ContentId`. It does not duplicate the packet or
/// prose content store; lookup is by intent name/version or by normalized
/// source id.
#[derive(Clone, Debug, Default)]
pub struct IntentLibrary {
    intents: BTreeMap<IntentKey, CompiledIntent>,
    source_index: BTreeMap<ContentId, BTreeSet<IntentKey>>,
}

impl IntentLibrary {
    /// Builds an empty intent library index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores or updates an intent record at its explicit name/version key.
    ///
    /// A packet change at an existing key is rejected because packet changes
    /// must become a new version. A stored golden record is immutable except
    /// for idempotent re-storage of the same record.
    pub fn store(&mut self, intent: CompiledIntent) -> Result<()> {
        let key = IntentKey::new(&intent.name, intent.version);
        if let Some(existing) = self.intents.get(&key) {
            if existing.status == IntentStatus::Golden && existing != &intent {
                return Err(Error::Eval(format!(
                    "golden intent {} v{} is immutable",
                    key.name, key.version
                )));
            }
            if existing.packet != intent.packet {
                return Err(Error::Eval(format!(
                    "compiled intent {} v{} changes packet; store a new version",
                    key.name, key.version
                )));
            }
        }

        if let Some(existing) = self.intents.insert(key.clone(), intent.clone()) {
            self.remove_source_key(&existing.source, &key);
        }
        self.source_index
            .entry(intent.source.clone())
            .or_default()
            .insert(key);
        Ok(())
    }

    /// Fetches an intent by exact name and version.
    pub fn fetch(&self, name: &Symbol, version: u32) -> Option<&CompiledIntent> {
        self.intents.get(&IntentKey::new(name, version))
    }

    /// Returns all intents whose normalized source id matches `source`.
    pub fn by_source(&self, source: &ContentId) -> Vec<&CompiledIntent> {
        self.source_index
            .get(source)
            .into_iter()
            .flat_map(|keys| keys.iter())
            .filter_map(|key| self.intents.get(key))
            .collect()
    }

    /// Returns the highest-version golden intent for `source`, when one exists.
    pub fn golden_by_source(&self, source: &ContentId) -> Option<&CompiledIntent> {
        self.by_source(source)
            .into_iter()
            .filter(|intent| intent.status == IntentStatus::Golden)
            .max_by_key(|intent| intent.version)
    }

    /// Returns the next version number available for `name`.
    pub fn next_version(&self, name: &Symbol) -> u32 {
        self.intents
            .keys()
            .filter(|key| &key.name == name)
            .map(|key| key.version)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    pub(crate) fn store_resolved(&mut self, mut intent: CompiledIntent) -> Result<CompiledIntent> {
        intent.version = self
            .version_for_packet(&intent.name, &intent.source, &intent.packet)
            .unwrap_or_else(|| self.next_version(&intent.name));
        self.store(intent.clone())?;
        Ok(intent)
    }

    fn version_for_packet(
        &self,
        name: &Symbol,
        source: &ContentId,
        packet: &ContentId,
    ) -> Option<u32> {
        self.intents.iter().find_map(|(key, intent)| {
            (&key.name == name && &intent.source == source && &intent.packet == packet)
                .then_some(key.version)
        })
    }

    fn remove_source_key(&mut self, source: &ContentId, key: &IntentKey) {
        if let Some(keys) = self.source_index.get_mut(source) {
            keys.remove(key);
            if keys.is_empty() {
                self.source_index.remove(source);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct IntentKey {
    name: Symbol,
    version: u32,
}

impl IntentKey {
    fn new(name: &Symbol, version: u32) -> Self {
        Self {
            name: name.clone(),
            version,
        }
    }
}
