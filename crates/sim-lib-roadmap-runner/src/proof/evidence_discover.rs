// SPDX-License-Identifier: MPL-2.0
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Discovery of generate, standalone, workspace, and package rows.

use std::collections::BTreeMap;

use super::DiscoveredDefinition;
use crate::ProofError;

/// Reads generate, standalone, workspace, and package rows from sealed text.
pub fn discover_definitions(text: &str) -> Result<Vec<DiscoveredDefinition>, ProofError> {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let header = lines.next().unwrap_or("");
    if header != "sim-world-inputs-v1" {
        return Err(ProofError::Invalid(
            "sealed text has no input header".into(),
        ));
    }
    let mut domains = BTreeMap::<String, String>::new();
    let mut pending = Vec::new();
    for line in lines {
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.first().copied() {
            Some("domain") if parts.len() == 5 => {
                domains.insert(parts[1].to_owned(), parts[3].to_owned());
            }
            Some("spell") if parts.len() == 4 => {
                let id = canonicalize_root(parts[1]);
                domains.entry(id).or_insert_with(|| parts[2].to_owned());
            }
            Some("root-package" | "member" | "generated") => pending.push(parts),
            _ => {}
        }
    }
    let compiling = domains
        .iter()
        .filter(|(_, role)| role.as_str() == "compiling")
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    let mut members = Vec::new();
    let mut generated = Vec::new();
    let mut roots = Vec::new();
    for parts in pending {
        match parts[0] {
            "root-package" if parts.len() == 2 => roots.push(parts),
            "generated" if parts.len() == 5 => generated.push(parts),
            "member" if parts.len() == 9 || parts.len() == 10 => members.push(parts),
            "root-package" | "member" | "generated" => {
                return Err(ProofError::Invalid("sealed definition is malformed".into()));
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    if !roots.is_empty() && compiling.len() == 1 {
        for parts in &roots {
            let version = root_version(&members, &compiling[0], parts[1]);
            let semantic = match &version {
                Some(version) => identity("package", &[&compiling[0], parts[1], version]),
                None => identity("package", &[&compiling[0], parts[1]]),
            };
            found.push(definition("package", &compiling[0], parts[1], semantic, ""));
        }
    }
    for parts in &generated {
        require_domain(&domains, parts[1])?;
        found.push(definition(
            "generate",
            parts[1],
            parts[2],
            identity("generate", &[parts[1], parts[2]]),
            parts[4],
        ));
    }
    for parts in &members {
        let role = require_domain(&domains, parts[1])?;
        if let Some(class) = member_class(parts[2], parts[7], role) {
            found.push(definition(
                class,
                parts[1],
                parts[3],
                identity(class, &[parts[1], parts[2], parts[3], parts[4], parts[5]]),
                parts[6],
            ));
        }
    }
    found.sort_by(|left, right| left.semantic.cmp(&right.semantic));
    Ok(found)
}

fn definition(
    class: &'static str,
    domain: &str,
    name: &str,
    semantic: String,
    storage: &str,
) -> DiscoveredDefinition {
    DiscoveredDefinition {
        class,
        domain: domain.to_owned(),
        name: name.to_owned(),
        semantic,
        storage: storage.to_owned(),
    }
}

fn require_domain<'a>(
    domains: &'a BTreeMap<String, String>,
    id: &str,
) -> Result<&'a str, ProofError> {
    domains
        .get(id)
        .map(String::as_str)
        .ok_or_else(|| ProofError::Invalid(format!("unknown domain {id}")))
}

fn root_version(members: &[Vec<&str>], domain: &str, name: &str) -> Option<String> {
    let mut versions = members
        .iter()
        .filter(|parts| {
            parts[1] == domain
                && parts[2] == "path"
                && parts[3] == name
                && parts[7] == "path-content"
        })
        .map(|parts| parts[4])
        .collect::<Vec<_>>();
    versions.sort();
    versions.dedup();
    if versions.is_empty() {
        None
    } else {
        Some(identity_fields(&versions))
    }
}

// Source URLs contain colons, so each field is length-prefixed.
fn identity(class: &str, fields: &[&str]) -> String {
    let mut parts = vec![class];
    parts.extend(fields);
    identity_fields(&parts)
}

fn identity_fields(fields: &[&str]) -> String {
    let mut out = String::new();
    for field in fields {
        out.push_str(&field.len().to_string());
        out.push(':');
        out.push_str(field);
        out.push(';');
    }
    out
}

fn canonicalize_root(raw: &str) -> String {
    let mut path = raw.replace('\\', "/");
    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }
    if let Some(stripped) = path.strip_suffix("/.") {
        path = stripped.to_owned();
    }
    if path.is_empty() {
        ".".to_owned()
    } else {
        path
    }
}

fn member_class(origin: &str, kind: &str, role: &str) -> Option<&'static str> {
    // Unverified and mismatched rows are observations, not definitions.
    if origin == "stub" || origin == "absent" {
        return None;
    }
    if matches!(kind, "none" | "version-mismatch" | "corrupt") {
        return None;
    }
    if kind == "path-content" && role == "compiling" {
        Some("workspace")
    } else if kind == "path-content" && role == "enumerated" {
        Some("standalone")
    } else if (origin == "registry" || origin == "git") && (kind == "registry" || kind == "git") {
        Some("package")
    } else {
        None
    }
}
