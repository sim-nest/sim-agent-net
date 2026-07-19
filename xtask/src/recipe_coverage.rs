use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn check_publishable_recipe_coverage(root: &Path) -> Result<usize, Vec<String>> {
    let manifest = root.join("Cargo.toml");
    if !manifest.is_file() {
        return Ok(0);
    }

    let members = workspace_members(&manifest).map_err(|err| vec![err])?;
    let mut publishable_packages = 0usize;
    let mut errors = Vec::new();
    for member in members {
        let package_root = root.join(&member);
        let package = package_info(&package_root).map_err(|err| vec![err])?;
        if !package.publish {
            continue;
        }
        publishable_packages += 1;
        if has_recipe_manifest(&package_root.join("recipes")).map_err(|err| vec![err])? {
            continue;
        }
        match package.recipe_policy {
            Some(policy) if policy.kind == "exempt" && !policy.reason.trim().is_empty() => {}
            Some(policy) if policy.kind == "exempt" => errors.push(format!(
                "{}: recipe coverage exemption requires a non-empty reason",
                slash(&package.manifest)
            )),
            Some(policy) => errors.push(format!(
                "{}: unsupported recipe coverage policy `{}`",
                slash(&package.manifest),
                policy.kind
            )),
            None => errors.push(format!(
                "{}: publishable package `{}` needs a recipes/ book or an explicit recipe coverage exemption",
                slash(&package.manifest),
                package.name
            )),
        }
    }

    if errors.is_empty() {
        Ok(publishable_packages)
    } else {
        Err(errors)
    }
}

fn workspace_members(manifest: &Path) -> Result<Vec<PathBuf>, String> {
    let text = fs::read_to_string(manifest)
        .map_err(|err| format!("read {}: {err}", manifest.display()))?;
    let mut in_workspace = false;
    let mut collecting_members = false;
    let mut members = Vec::new();
    for raw in text.lines() {
        let stripped = strip_comment(raw);
        let line = stripped.trim();
        if line.starts_with('[') {
            in_workspace = line == "[workspace]";
            collecting_members = false;
        }
        if collecting_members {
            members.extend(quoted_strings(line).into_iter().map(PathBuf::from));
            if line.contains(']') {
                collecting_members = false;
            }
            continue;
        }
        if in_workspace
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "members"
        {
            members.extend(quoted_strings(value).into_iter().map(PathBuf::from));
            if !value.contains(']') {
                collecting_members = true;
            }
        }
    }
    if members.is_empty() {
        return Err(format!("{}: workspace has no members", manifest.display()));
    }
    Ok(members)
}

fn package_info(root: &Path) -> Result<PackageInfo, String> {
    let manifest = root.join("Cargo.toml");
    let text = fs::read_to_string(&manifest)
        .map_err(|err| format!("read {}: {err}", manifest.display()))?;
    let mut section = String::new();
    let mut name = None;
    let mut publish = true;
    let mut policy_kind = None;
    let mut policy_reason = None;

    for raw in text.lines() {
        let stripped = strip_comment(raw);
        let line = stripped.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            section = line.trim_matches(&['[', ']'][..]).to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        match section.as_str() {
            "package" if key == "name" => name = string_literal(value),
            "package" if key == "publish" => publish = value != "false",
            "package.metadata.sim-recipes" if key == "policy" => {
                policy_kind = string_literal(value)
            }
            "package.metadata.sim-recipes" if key == "reason" => {
                policy_reason = string_literal(value)
            }
            _ => {}
        }
    }

    let recipe_policy = policy_kind.map(|kind| RecipePolicy {
        kind,
        reason: policy_reason.unwrap_or_default(),
    });
    Ok(PackageInfo {
        name: name.ok_or_else(|| format!("{}: missing package name", manifest.display()))?,
        manifest,
        publish,
        recipe_policy,
    })
}

fn has_recipe_manifest(root: &Path) -> Result<bool, String> {
    if !root.is_dir() {
        return Ok(false);
    }
    for entry in fs::read_dir(root).map_err(|err| format!("read {}: {err}", root.display()))? {
        let entry = entry.map_err(|err| format!("read {}: {err}", root.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("stat {}: {err}", path.display()))?;
        if file_type.is_dir() {
            if has_recipe_manifest(&path)? {
                return Ok(true);
            }
        } else if path.file_name().and_then(|name| name.to_str()) == Some("recipe.toml") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn string_literal(value: &str) -> Option<String> {
    quoted_strings(value).into_iter().next()
}

fn quoted_strings(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in value.chars() {
        if in_string {
            if escaped {
                current.push(ch);
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                out.push(std::mem::take(&mut current));
                in_string = false;
            } else {
                current.push(ch);
            }
        } else if ch == '"' {
            in_string = true;
        }
    }
    out
}

fn strip_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut escaped = false;
    for (idx, c) in line.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
        } else if c == '#' {
            return &line[..idx];
        }
    }
    line
}

fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

struct PackageInfo {
    name: String,
    manifest: PathBuf,
    publish: bool,
    recipe_policy: Option<RecipePolicy>,
}

struct RecipePolicy {
    kind: String,
    reason: String,
}
