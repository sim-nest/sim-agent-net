use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CheckSummary {
    pub(crate) checked_recipes: usize,
    pub(crate) agent30_recipes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Str(String),
    Int(i64),
    Bool,
    Array(Vec<String>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RecipeDoc {
    top: BTreeMap<String, Value>,
    expects: Vec<BTreeMap<String, Value>>,
}

pub(crate) fn check_repo(root: &Path) -> Result<CheckSummary, String> {
    let mut recipe_paths = Vec::new();
    collect_recipe_manifests(root, root, &mut recipe_paths)?;

    let mut summary = CheckSummary::default();
    let mut errors = Vec::new();
    for path in recipe_paths {
        match check_recipe(root, &path) {
            Ok(recipe_summary) => {
                summary.checked_recipes += recipe_summary.checked_recipes;
                summary.agent30_recipes += recipe_summary.agent30_recipes;
            }
            Err(mut recipe_errors) => errors.append(&mut recipe_errors),
        }
    }

    if errors.is_empty() {
        Ok(summary)
    } else {
        Err(format!(
            "recipe assertion check failed:\n{}",
            errors.join("\n")
        ))
    }
}

fn collect_recipe_manifests(
    root: &Path,
    dir: &Path,
    paths: &mut Vec<PathBuf>,
) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|err| format!("read {}: {err}", dir.display()))? {
        let entry = entry.map_err(|err| format!("read {}: {err}", dir.display()))?;
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("stat {}: {err}", path.display()))?;
        if file_type.is_dir() {
            if matches!(
                file_name.as_ref(),
                ".git" | ".meta-workspace" | "target" | "generated-reports" | "split-reports"
            ) {
                continue;
            }
            collect_recipe_manifests(root, &path, paths)?;
        } else if file_name == "recipe.toml"
            && path
                .components()
                .any(|component| component.as_os_str() == "recipes")
        {
            paths.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
    paths.sort();
    Ok(())
}

fn check_recipe(root: &Path, relative_path: &Path) -> Result<CheckSummary, Vec<String>> {
    let path = root.join(relative_path);
    let text = fs::read_to_string(&path)
        .map_err(|err| vec![format!("{}: cannot read: {err}", slash(relative_path))])?;
    let doc =
        parse_recipe_doc(&text).map_err(|err| vec![format!("{}: {err}", slash(relative_path))])?;

    let dir = relative_path.parent().ok_or_else(|| {
        vec![format!(
            "{}: missing recipe directory",
            slash(relative_path)
        )]
    })?;
    let mut errors = Vec::new();
    let mut checked = false;
    let mut agent30 = false;

    if check_assertions(root, relative_path, dir, &doc, &mut errors) {
        checked = true;
    }
    if is_agent30_recipe(dir) {
        agent30 = !is_agent30_capstone(&doc);
        checked = true;
        check_agent30_metadata(relative_path, dir, &doc, &mut errors);
    }

    if errors.is_empty() {
        Ok(CheckSummary {
            checked_recipes: usize::from(checked),
            agent30_recipes: usize::from(agent30),
        })
    } else {
        Err(errors)
    }
}

fn check_assertions(
    root: &Path,
    recipe_path: &Path,
    recipe_dir: &Path,
    doc: &RecipeDoc,
    errors: &mut Vec<String>,
) -> bool {
    let mut checked = false;
    let tags = optional_array(doc, "tags").unwrap_or_default();
    let capabilities = optional_array(doc, "capabilities").unwrap_or_default();

    if let Some(assert_tags) = optional_array(doc, "assert_tags") {
        checked = true;
        for tag in assert_tags {
            if !tags.contains(&tag) {
                errors.push(format!(
                    "{}: missing asserted tag `{tag}`",
                    slash(recipe_path)
                ));
            }
        }
    }
    if let Some(assert_capabilities) = optional_array(doc, "assert_capabilities") {
        checked = true;
        for capability in assert_capabilities {
            if !capabilities.contains(&capability) {
                errors.push(format!(
                    "{}: missing asserted capability `{capability}`",
                    slash(recipe_path)
                ));
            }
        }
    }
    if let Some(assert_codec) = optional_string(doc, "assert_setup_codec") {
        checked = true;
        match optional_string(doc, "codec") {
            Some(codec) if codec == assert_codec => {}
            Some(codec) => errors.push(format!(
                "{}: asserted setup codec `{assert_codec}` does not match codec `{codec}`",
                slash(recipe_path)
            )),
            None => errors.push(format!("{}: missing `codec`", slash(recipe_path))),
        }
    }
    if let Some(assert_shape) = optional_string(doc, "assert_descriptor_shape") {
        checked = true;
        match optional_string(doc, "descriptor_shape") {
            Some(shape) if shape == assert_shape => {}
            Some(shape) => errors.push(format!(
                "{}: asserted descriptor shape `{assert_shape}` does not match `{shape}`",
                slash(recipe_path)
            )),
            None => errors.push(format!(
                "{}: missing `descriptor_shape`",
                slash(recipe_path)
            )),
        }
    }
    if let Some(expected) = optional_string(doc, "expected") {
        checked = true;
        check_expected_file(root, recipe_path, recipe_dir, &expected, doc, errors);
    }

    checked
}

fn check_expected_file(
    root: &Path,
    recipe_path: &Path,
    recipe_dir: &Path,
    expected: &str,
    doc: &RecipeDoc,
    errors: &mut Vec<String>,
) {
    let expected_path = root.join(recipe_dir).join(expected);
    let expected_text = match fs::read_to_string(&expected_path) {
        Ok(text) => text.trim_end().to_string(),
        Err(err) => {
            errors.push(format!(
                "{}: expected file `{expected}` cannot be read: {err}",
                slash(recipe_path)
            ));
            return;
        }
    };
    let matches_expect = doc.expects.iter().any(|table| {
        matches!(
            table.get("result"),
            Some(Value::Str(result)) if result.trim_end() == expected_text
        )
    });
    if !matches_expect {
        errors.push(format!(
            "{}: expected file `{expected}` is not mirrored by any `[[expect]].result`",
            slash(recipe_path)
        ));
    }
}

fn check_agent30_metadata(
    recipe_path: &Path,
    recipe_dir: &Path,
    doc: &RecipeDoc,
    errors: &mut Vec<String>,
) {
    let tags = optional_array(doc, "tags").unwrap_or_default();
    let id = required_string(doc, recipe_path, "id", errors).unwrap_or_default();
    if !id.starts_with("a30-") {
        errors.push(format!(
            "{}: 30-agents recipe id `{id}` must start with `a30-`",
            slash(recipe_path)
        ));
    }
    let dir_id = recipe_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if !id.is_empty() && id != dir_id {
        errors.push(format!(
            "{}: recipe id `{id}` must match directory `{dir_id}`",
            slash(recipe_path)
        ));
    }

    if is_agent30_capstone(doc) {
        if !tags.contains(&"outside-30-count".to_owned()) {
            errors.push(format!(
                "{}: capstone recipes must carry `outside-30-count`",
                slash(recipe_path)
            ));
        }
        if doc.top.contains_key("recipe_number") {
            errors.push(format!(
                "{}: capstone recipes outside the 30 count must omit `recipe_number`",
                slash(recipe_path)
            ));
        }
    } else {
        let recipe_number = required_int(doc, recipe_path, "recipe_number", errors);
        if matches!(recipe_number, Some(number) if number <= 0) {
            errors.push(format!(
                "{}: `recipe_number` must be positive",
                slash(recipe_path)
            ));
        }
    }
    let source_chapter = required_int(doc, recipe_path, "source_chapter", errors);
    if let Some(chapter) = source_chapter {
        let chapter_tag = format!("chapter-{chapter:02}");
        if !tags.contains(&chapter_tag) {
            errors.push(format!(
                "{}: missing source chapter tag `{chapter_tag}`",
                slash(recipe_path)
            ));
        }
    }

    if let Some(family) = required_string(doc, recipe_path, "architecture_family", errors)
        && !tags.contains(&family)
    {
        errors.push(format!(
            "{}: architecture family `{family}` must also be a tag",
            slash(recipe_path)
        ));
    }
    if let Some(posture) = required_string(doc, recipe_path, "safety_posture", errors)
        && !tags.contains(&posture)
    {
        errors.push(format!(
            "{}: safety posture `{posture}` must also be a tag",
            slash(recipe_path)
        ));
    }
    let _ = required_string(doc, recipe_path, "runner_mode", errors);
    match optional_array(doc, "capabilities") {
        Some(values) if values.is_empty() => errors.push(format!(
            "{}: `capabilities` must list at least one capability",
            slash(recipe_path)
        )),
        Some(_) => {}
        None => errors.push(format!("{}: missing `capabilities`", slash(recipe_path))),
    }
}

fn is_agent30_recipe(dir: &Path) -> bool {
    dir.components()
        .any(|component| component.as_os_str() == "30-agents")
}

fn is_agent30_capstone(doc: &RecipeDoc) -> bool {
    optional_array(doc, "tags")
        .unwrap_or_default()
        .iter()
        .any(|tag| tag == "capstone")
}

fn required_string(
    doc: &RecipeDoc,
    recipe_path: &Path,
    key: &str,
    errors: &mut Vec<String>,
) -> Option<String> {
    match optional_string(doc, key) {
        Some(value) if value.is_empty() => {
            errors.push(format!("{}: `{key}` must not be empty", slash(recipe_path)));
            None
        }
        Some(value) => Some(value),
        None => {
            errors.push(format!("{}: missing `{key}`", slash(recipe_path)));
            None
        }
    }
}

fn required_int(
    doc: &RecipeDoc,
    recipe_path: &Path,
    key: &str,
    errors: &mut Vec<String>,
) -> Option<i64> {
    match doc.top.get(key) {
        Some(Value::Int(value)) => Some(*value),
        Some(other) => {
            errors.push(format!(
                "{}: `{key}` must be an integer, found {}",
                slash(recipe_path),
                other.type_name()
            ));
            None
        }
        None => {
            errors.push(format!("{}: missing `{key}`", slash(recipe_path)));
            None
        }
    }
}

fn optional_string(doc: &RecipeDoc, key: &str) -> Option<String> {
    match doc.top.get(key) {
        Some(Value::Str(value)) => Some(value.clone()),
        _ => None,
    }
}

fn optional_array(doc: &RecipeDoc, key: &str) -> Option<Vec<String>> {
    match doc.top.get(key) {
        Some(Value::Array(values)) => Some(values.clone()),
        _ => None,
    }
}

fn parse_recipe_doc(text: &str) -> Result<RecipeDoc, String> {
    let mut doc = RecipeDoc::default();
    let mut in_expect: Option<usize> = None;
    for (idx, raw) in text.lines().enumerate() {
        let line_no = idx + 1;
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line == "[[expect]]" {
            doc.expects.push(BTreeMap::new());
            in_expect = Some(doc.expects.len() - 1);
            continue;
        }
        if line.starts_with('[') {
            return Err(format!("line {line_no}: unsupported table `{line}`"));
        }
        let (key, value) =
            parse_assignment(line).map_err(|err| format!("line {line_no}: {err}"))?;
        if let Some(expect_idx) = in_expect {
            doc.expects[expect_idx].insert(key, value);
        } else {
            doc.top.insert(key, value);
        }
    }
    Ok(doc)
}

fn parse_assignment(line: &str) -> Result<(String, Value), String> {
    let eq = line.find('=').ok_or("expected `key = value`")?;
    let key = line[..eq].trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!("invalid key `{key}`"));
    }
    Ok((key.to_string(), parse_value(line[eq + 1..].trim())?))
}

fn parse_value(raw: &str) -> Result<Value, String> {
    if raw.starts_with('"') {
        let (value, rest) = take_string(raw)?;
        if !rest.trim().is_empty() {
            return Err(format!("trailing text after string `{}`", rest.trim()));
        }
        return Ok(Value::Str(value));
    }
    if raw.starts_with('[') {
        return parse_array(raw).map(Value::Array);
    }
    if raw == "true" || raw == "false" {
        return Ok(Value::Bool);
    }
    raw.parse::<i64>()
        .map(Value::Int)
        .map_err(|_| format!("unrecognized value `{raw}`"))
}

fn parse_array(raw: &str) -> Result<Vec<String>, String> {
    let inner = raw
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .ok_or("unterminated array")?;
    let mut values = Vec::new();
    let mut rest = inner.trim();
    while !rest.is_empty() {
        let (value, after) = take_string(rest)?;
        values.push(value);
        rest = after.trim_start();
        if let Some(after_comma) = rest.strip_prefix(',') {
            rest = after_comma.trim_start();
        } else if !rest.is_empty() {
            return Err(format!("expected `,` in array, found `{rest}`"));
        }
    }
    Ok(values)
}

fn take_string(raw: &str) -> Result<(String, &str), String> {
    let bytes = raw.as_bytes();
    if bytes.first() != Some(&b'"') {
        return Err("expected quoted string".to_string());
    }
    let mut value = String::new();
    let mut chars = raw.char_indices().skip(1);
    while let Some((idx, c)) = chars.next() {
        match c {
            '"' => return Ok((value, &raw[idx + 1..])),
            '\\' => match chars.next() {
                Some((_, 'n')) => value.push('\n'),
                Some((_, 't')) => value.push('\t'),
                Some((_, '"')) => value.push('"'),
                Some((_, '\\')) => value.push('\\'),
                Some((_, other)) => value.push(other),
                None => return Err("unterminated escape".to_string()),
            },
            other => value.push(other),
        }
    }
    Err("unterminated string".to_string())
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

impl Value {
    fn type_name(&self) -> &'static str {
        match self {
            Self::Str(_) => "string",
            Self::Int(_) => "integer",
            Self::Bool => "bool",
            Self::Array(_) => "array",
        }
    }
}
