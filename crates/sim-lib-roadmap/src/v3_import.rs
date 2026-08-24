use crate::*;
use sim_codec_doc::{
    Inline, MarkdownBackend, MarkupBackend, MarkupBlock, MarkupDecodeOptions, Span,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImportError {
    Markdown(String),
    Invalid(String),
    Reference(String),
}
impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Markdown(s) | Self::Invalid(s) | Self::Reference(s) => f.write_str(s),
        }
    }
}
impl std::error::Error for ImportError {}

pub struct V3Importer<'a> {
    documents: &'a BTreeMap<String, Vec<u8>>,
}
impl<'a> V3Importer<'a> {
    pub fn new(documents: &'a BTreeMap<String, Vec<u8>>) -> Self {
        Self { documents }
    }
    pub fn import(&self, path: &str, source: &str) -> Result<ImportedRoadmap, ImportError> {
        validate_path(path)?;
        let (doc, _) = MarkdownBackend
            .decode(
                source,
                &MarkupDecodeOptions {
                    preserve_source: true,
                    preserve_raw: true,
                },
            )
            .map_err(|e| ImportError::Markdown(e.to_string()))?;
        let mut observation = LegacyObservation::default();
        for line in source.lines().take_while(|line| !line.starts_with("### ")) {
            observe_header(line, &mut observation);
        }
        let mut phases = Vec::new();
        let mut current: Option<NativePhase> = None;
        let mut seen = BTreeSet::new();
        let title = doc.title.unwrap_or_else(|| path.to_owned());
        for block in doc.blocks {
            match block {
                MarkupBlock::Heading {
                    level: 3,
                    text,
                    span,
                    ..
                } => {
                    if let Some(phase) = current.take() {
                        phases.push(phase);
                    }
                    let heading = plain(&text);
                    let (id, name) = parse_phase_heading(&heading).ok_or_else(|| {
                        ImportError::Invalid(format!(
                            "phase heading at {} lacks an id",
                            start(&span)
                        ))
                    })?;
                    if !seen.insert(id.clone()) {
                        return Err(ImportError::Invalid(format!("duplicate phase {id}")));
                    }
                    current = Some(NativePhase {
                        id,
                        title: name,
                        span: range(path, span)?,
                        dependencies: vec![],
                        checkpoints: vec![],
                        origins: vec![],
                        guides: vec![],
                        children: vec![],
                    });
                }
                block => {
                    let text = block_text(&block);
                    if current.is_none() {
                        continue;
                    }
                    let phase = current.as_mut().unwrap();
                    if text.contains('\n') && text.lines().any(|line| field(line).is_some()) {
                        let span = block_span(&block);
                        for line in text.lines() {
                            if let Some(status) = line.strip_prefix("Status:") {
                                observation
                                    .phase_status
                                    .insert(phase.id.clone(), status.trim().to_owned());
                            } else if let Some((key, value)) = field(line) {
                                import_field(self, path, phase, key, value, span.clone())?;
                            }
                        }
                        continue;
                    }
                    if let Some(status) = text.strip_prefix("Status:") {
                        observation
                            .phase_status
                            .insert(phase.id.clone(), status.trim().to_owned());
                        continue;
                    }
                    if let Some((key, value)) = field(&text) {
                        import_field(self, path, phase, key, value, block_span(&block))?;
                    } else if let MarkupBlock::List { items, span, .. } = &block {
                        for (index, item) in items.iter().enumerate() {
                            let item_text =
                                item.iter().map(block_text).collect::<Vec<_>>().join(" ");
                            let checked =
                                item_text.starts_with("[x] ") || item_text.starts_with("[X] ");
                            let task = item_text
                                .strip_prefix("[ ] ")
                                .or_else(|| item_text.strip_prefix("[x] "))
                                .or_else(|| item_text.strip_prefix("[X] "));
                            if let Some(task) = task {
                                phase.checkpoints.push(Checkpoint {
                                    text: task.to_owned(),
                                    span: range(path, span.clone())?,
                                });
                                if checked {
                                    observation
                                        .checked_tasks
                                        .entry(phase.id.clone())
                                        .or_default()
                                        .push(index + 1);
                                }
                            } else {
                                phase
                                    .guides
                                    .push(guide(path, item_text, None, span.clone())?);
                            }
                        }
                    } else if let MarkupBlock::CodeBlock { lang, code, span } = block {
                        phase.guides.push(guide(path, code, lang, span)?);
                    } else if !text.trim().is_empty() {
                        phase
                            .guides
                            .push(guide(path, text, None, block_span(&block))?);
                    }
                }
            }
        }
        if let Some(phase) = current {
            phases.push(phase);
        }
        if phases.is_empty() {
            return Err(ImportError::Invalid(
                "document has no level-three v3 phases".into(),
            ));
        }
        let root = NativePhase {
            id: "root".into(),
            title: title.clone(),
            span: SourceRange {
                document: path.into(),
                start: 0,
                end: source.len(),
            },
            dependencies: vec![],
            checkpoints: vec![],
            origins: vec![],
            guides: vec![],
            children: phases,
        };
        Ok(ImportedRoadmap {
            roadmap: NativeRoadmap {
                title,
                root,
                limitations: vec![
                    "legacy prose and snippets are ungrounded".into(),
                    "runtime lifecycle is observation-only".into(),
                ],
            },
            observation,
        })
    }
    fn origin(
        &self,
        owner: &str,
        key: &str,
        reference: &str,
        span: Option<Span>,
    ) -> Result<Origin, ImportError> {
        let (located, fragment) = reference
            .split_once('#')
            .map_or((reference, None), |(p, f)| (p, Some(f.to_owned())));
        let (path, expected_id) = located
            .split_once('@')
            .map_or((located, None), |(p, id)| (p, Some(id)));
        validate_path(path)?;
        let bytes = self.documents.get(path).ok_or_else(|| {
            ImportError::Reference(format!("missing caller-supplied bytes for {path}"))
        })?;
        let relation = match key {
            "SOURCE_PHASE" => OriginRelation::Source,
            "REVIEW_PHASE" => OriginRelation::Review,
            _ => OriginRelation::Merged,
        };
        let content_id = format!("fnv1a64:{:016x}", stable_hash(bytes));
        if expected_id.is_some_and(|expected| expected != content_id) {
            return Err(ImportError::Reference(format!(
                "stale content id for {path}: expected {expected_id:?}, got {content_id}"
            )));
        }
        Ok(Origin {
            relation,
            path: path.into(),
            fragment,
            content_id,
            span: range(owner, span)?,
        })
    }
}

fn import_field(
    importer: &V3Importer<'_>,
    path: &str,
    phase: &mut NativePhase,
    key: &str,
    value: &str,
    span: Option<Span>,
) -> Result<(), ImportError> {
    match key {
        "SOURCE_PHASE" | "REVIEW_PHASE" | "MERGED_SOURCE_PHASES" => {
            for reference in split_refs(value) {
                let origin = importer.origin(path, key, reference, span.clone())?;
                if phase.origins.iter().any(|seen| {
                    seen.relation == origin.relation
                        && seen.path == origin.path
                        && seen.fragment == origin.fragment
                }) {
                    return Err(ImportError::Reference(format!(
                        "duplicate {key} reference {reference}"
                    )));
                }
                phase.origins.push(origin);
            }
        }
        "DEPENDS_ON" | "AFTER" => {
            for target in value.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                phase.dependencies.push(PhaseEdge {
                    target: target.to_owned(),
                    kind: if key == "AFTER" {
                        EdgeKind::After
                    } else {
                        EdgeKind::Requires
                    },
                    span: range(path, span.clone())?,
                });
            }
        }
        _ => phase
            .guides
            .push(guide(path, format!("{key}: {value}"), None, span)?),
    }
    Ok(())
}

fn validate_path(path: &str) -> Result<(), ImportError> {
    if path.is_empty() || path.starts_with('/') || path.split('/').any(|p| p == ".." || p == ".") {
        Err(ImportError::Reference(format!(
            "reference path escapes caller map: {path}"
        )))
    } else {
        Ok(())
    }
}
fn parse_phase_heading(s: &str) -> Option<(String, String)> {
    let s = s.trim_start_matches(['[', ']', ' ', 'x', 'X']);
    let (id, title) = s.split_once(" - ")?;
    Some((id.trim().into(), title.trim().into()))
}
fn plain(v: &[Inline]) -> String {
    v.iter()
        .map(|i| match i {
            Inline::Text(s) | Inline::Code(s) => s.clone(),
            Inline::Emph(x) | Inline::Strong(x) => plain(x),
            Inline::Link { label, .. } => plain(label),
            Inline::Math(m) => m.text.clone(),
            Inline::Raw { text, .. } => text.clone(),
        })
        .collect()
}
fn block_text(b: &MarkupBlock) -> String {
    match b {
        MarkupBlock::Heading { text, .. } => plain(text),
        MarkupBlock::Paragraph { content, .. } => plain(content),
        MarkupBlock::CodeBlock { code, .. } | MarkupBlock::Raw { text: code, .. } => code.clone(),
        MarkupBlock::List { items, .. } => items
            .iter()
            .flatten()
            .map(block_text)
            .collect::<Vec<_>>()
            .join(" "),
        MarkupBlock::Table { header, rows, .. } => header
            .iter()
            .chain(rows.iter().flatten())
            .map(|x| plain(x))
            .collect::<Vec<_>>()
            .join(" | "),
        _ => String::new(),
    }
}
fn block_span(b: &MarkupBlock) -> Option<Span> {
    match b {
        MarkupBlock::Heading { span, .. }
        | MarkupBlock::Paragraph { span, .. }
        | MarkupBlock::CodeBlock { span, .. }
        | MarkupBlock::MathBlock { span, .. }
        | MarkupBlock::Quote { span, .. }
        | MarkupBlock::List { span, .. }
        | MarkupBlock::Table { span, .. }
        | MarkupBlock::Figure { span, .. }
        | MarkupBlock::Raw { span, .. } => span.clone(),
    }
}
fn range(path: &str, span: Option<Span>) -> Result<SourceRange, ImportError> {
    let s = span.ok_or_else(|| {
        ImportError::Invalid("MarkdownBackend omitted a required source span".into())
    })?;
    Ok(SourceRange {
        document: path.into(),
        start: s.start,
        end: s.end,
    })
}
fn start(span: &Option<Span>) -> usize {
    span.as_ref().map_or(0, |s| s.start)
}
fn field(s: &str) -> Option<(&str, &str)> {
    let (k, v) = s.split_once(':')?;
    k.chars()
        .all(|c| c.is_ascii_uppercase() || c == '_')
        .then_some((k, v.trim()))
}
fn split_refs(s: &str) -> impl Iterator<Item = &str> {
    s.split(',').map(str::trim).filter(|s| !s.is_empty())
}
fn guide(
    path: &str,
    text: String,
    language: Option<String>,
    span: Option<Span>,
) -> Result<GuideEntry, ImportError> {
    Ok(GuideEntry {
        text,
        language,
        span: range(path, span)?,
        grounded: false,
    })
}
fn observe_header(s: &str, o: &mut LegacyObservation) {
    if let Some(v) = s.strip_prefix("STATE:") {
        o.state = Some(v.trim().into())
    }
    if let Some(v) = s.strip_prefix("NEXT:") {
        o.next = Some(v.trim().into())
    }
}
