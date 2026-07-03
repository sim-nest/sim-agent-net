use std::collections::BTreeSet;

use crate::McpSurfaceCard;

/// Allow/deny filter that decides which surface rows reach a client.
///
/// Names are matched with simple `*` glob patterns. A row is allowed when it
/// matches no deny pattern and either the allow set is empty or it matches an
/// allow pattern.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct McpProfile {
    allow_names: BTreeSet<String>,
    deny_names: BTreeSet<String>,
}

impl McpProfile {
    /// Returns a profile that allows every row.
    pub fn all() -> Self {
        Self::default()
    }

    /// Builds a profile whose allow set is `names` and whose deny set is empty.
    pub fn allow_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            allow_names: names.into_iter().map(Into::into).collect(),
            deny_names: BTreeSet::new(),
        }
    }

    /// Builds a profile whose deny set is `names` and whose allow set is empty.
    pub fn deny_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            allow_names: BTreeSet::new(),
            deny_names: names.into_iter().map(Into::into).collect(),
        }
    }

    /// Returns the profile with `name` added to the allow set.
    pub fn with_allowed_name(mut self, name: impl Into<String>) -> Self {
        self.allow_names.insert(name.into());
        self
    }

    /// Returns the profile with `name` added to the deny set.
    pub fn with_denied_name(mut self, name: impl Into<String>) -> Self {
        self.deny_names.insert(name.into());
        self
    }

    /// Reports whether a row named `name` passes this profile.
    pub fn allows_name(&self, name: &str) -> bool {
        if matches_any(&self.deny_names, name) {
            return false;
        }
        self.allow_names.is_empty() || matches_any(&self.allow_names, name)
    }

    /// Reports whether `row` passes this profile.
    pub fn allows(&self, row: &McpSurfaceCard) -> bool {
        self.allows_name(&row.name)
    }
}

fn matches_any(patterns: &BTreeSet<String>, name: &str) -> bool {
    patterns.iter().any(|pattern| glob_matches(pattern, name))
}

fn glob_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" || pattern == name {
        return true;
    }
    if !pattern.contains('*') {
        return false;
    }
    let mut rest = name;
    let anchored_start = !pattern.starts_with('*');
    let anchored_end = !pattern.ends_with('*');
    let parts = pattern.split('*').filter(|part| !part.is_empty());
    let mut first = true;
    for part in parts {
        if first && anchored_start {
            let Some(tail) = rest.strip_prefix(part) else {
                return false;
            };
            rest = tail;
        } else {
            let Some(index) = rest.find(part) else {
                return false;
            };
            rest = &rest[index + part.len()..];
        }
        first = false;
    }
    !anchored_end || rest.is_empty()
}
