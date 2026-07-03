use std::{collections::BTreeMap, error::Error, fmt};

use sim_lib_numbers_stats::{bayesian_update_binary, entropy};
use sim_lib_rank::{EmbeddingStore, retrieve_ids};

const DEFAULT_RADAR_LIMIT: usize = 8;
const DEFAULT_QUERY_TEXT: &str = "atelier radar";

/// Source location owned by a sibling checkout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSpan {
    /// Repository name from `repos.toml`.
    pub repo: String,
    /// Path relative to the owning repository root.
    pub path: String,
    /// One-based source line.
    pub line: usize,
}

/// Query for ranked Atelier hints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadarQuery {
    /// Free-text query.
    pub text: String,
    /// Optional repository filter.
    pub repo: Option<String>,
    /// Optional crate filter.
    pub crate_name: Option<String>,
    /// Optional indexed source kind filter.
    pub kind: Option<String>,
    /// Optional capability filter.
    pub capability: Option<String>,
    /// Optional codec filter.
    pub codec: Option<String>,
    /// Optional agent-role filter.
    pub agent_role: Option<String>,
    /// Maximum number of hints to return.
    pub limit: usize,
}

impl RadarQuery {
    /// Builds a query with the default hint limit.
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            repo: None,
            crate_name: None,
            kind: None,
            capability: None,
            codec: None,
            agent_role: None,
            limit: DEFAULT_RADAR_LIMIT,
        }
    }

    fn search_text(&self) -> String {
        [
            self.text.as_str(),
            self.repo.as_deref().unwrap_or_default(),
            self.crate_name.as_deref().unwrap_or_default(),
            self.kind.as_deref().unwrap_or_default(),
            self.capability.as_deref().unwrap_or_default(),
            self.codec.as_deref().unwrap_or_default(),
            self.agent_role.as_deref().unwrap_or_default(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
    }
}

/// Chunk from the F1 constellation index that can be ranked by Radar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadarChunk {
    /// Stable chunk id from the index.
    pub chunk_id: String,
    /// One-line purpose or title.
    pub title: String,
    /// Owning source span.
    pub span: SourceSpan,
    /// Owning crate, when the index knows one.
    pub crate_name: Option<String>,
    /// Indexed source kind.
    pub kind: String,
    /// Indexed text used for retrieval.
    pub text: String,
    /// Capability labels inferred for this chunk.
    pub capabilities: Vec<String>,
    /// Codec labels inferred for this chunk.
    pub codecs: Vec<String>,
    /// Agent roles associated with this chunk.
    pub agent_roles: Vec<String>,
    /// Whether the span still resolves in the owning checkout.
    pub live: bool,
}

impl RadarChunk {
    /// Builds a live chunk with empty filters.
    pub fn new(
        chunk_id: impl Into<String>,
        title: impl Into<String>,
        span: SourceSpan,
        kind: impl Into<String>,
        text: impl Into<String>,
    ) -> Self {
        Self {
            chunk_id: chunk_id.into(),
            title: title.into(),
            span,
            crate_name: None,
            kind: kind.into(),
            text: text.into(),
            capabilities: Vec::new(),
            codecs: Vec::new(),
            agent_roles: Vec::new(),
            live: true,
        }
    }

    fn matches(&self, query: &RadarQuery) -> bool {
        matches_field(&self.span.repo, &query.repo)
            && matches_optional(self.crate_name.as_deref(), &query.crate_name)
            && matches_field(&self.kind, &query.kind)
            && matches_list(&self.capabilities, &query.capability)
            && matches_list(&self.codecs, &query.codec)
            && matches_list_or_text(&self.agent_roles, &self.text, &query.agent_role)
    }

    fn search_text(&self) -> String {
        [
            self.title.as_str(),
            self.kind.as_str(),
            self.crate_name.as_deref().unwrap_or_default(),
            self.text.as_str(),
            &self.capabilities.join(" "),
            &self.codecs.join(" "),
            &self.agent_roles.join(" "),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
    }
}

/// F1 chunks available to Radar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RadarIndex {
    /// Indexed chunks.
    pub chunks: Vec<RadarChunk>,
}

/// Ranked, confidence-scored hint for an Atelier operation.
#[derive(Clone, Debug, PartialEq)]
pub struct RadarHint {
    /// Stable chunk id from the index.
    pub chunk_id: String,
    /// One-line purpose.
    pub title: String,
    /// Live source span.
    pub span: SourceSpan,
    /// Capability labels attached to the chunk.
    pub capabilities: Vec<String>,
    /// Preferred codec form for viewing or editing the chunk.
    pub preferred_codec: Option<String>,
    /// F4-derived confidence in `0.0..=1.0`.
    pub confidence: f64,
}

/// Radar retrieval result.
#[derive(Clone, Debug, PartialEq)]
pub struct RadarReport {
    /// Ranked hints after filters and stale-span removal.
    pub hints: Vec<RadarHint>,
    /// Whether any matching chunk had a stale source span.
    pub stale_index: bool,
    /// Matching stale chunk ids dropped from the hint list.
    pub stale_chunk_ids: Vec<String>,
}

/// Result alias for Radar operations.
pub type RadarResult<T> = Result<T, RadarError>;

/// Error returned when a Radar index or query cannot be ranked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadarError {
    message: String,
}

impl RadarError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for RadarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for RadarError {}

/// Retrieves ranked hints by applying F2 nearest-neighbor ordering and F4 confidence.
pub fn retrieve_radar_hints(index: &RadarIndex, query: &RadarQuery) -> RadarResult<RadarReport> {
    let matching = index
        .chunks
        .iter()
        .filter(|chunk| chunk.matches(query))
        .collect::<Vec<_>>();
    let stale_chunk_ids = matching
        .iter()
        .filter(|chunk| !chunk.live)
        .map(|chunk| chunk.chunk_id.clone())
        .collect::<Vec<_>>();
    let live_chunks = matching
        .into_iter()
        .filter(|chunk| chunk.live)
        .collect::<Vec<_>>();
    if live_chunks.is_empty() {
        return Ok(RadarReport {
            hints: Vec::new(),
            stale_index: !stale_chunk_ids.is_empty(),
            stale_chunk_ids,
        });
    }

    let mut store = EmbeddingStore::new();
    let mut by_id = BTreeMap::new();
    for chunk in &live_chunks {
        store
            .insert(chunk.chunk_id.clone(), embedding_vec(&chunk.search_text()))
            .map_err(|err| RadarError::new(format!("rank index insert failed: {err}")))?;
        by_id.insert(chunk.chunk_id.as_str(), *chunk);
    }

    let query_embedding = embedding_vec(&query.search_text());
    let neighbors = retrieve_ids(
        &store,
        &query_embedding,
        live_chunks.iter().map(|chunk| chunk.chunk_id.as_str()),
        query.limit,
    )
    .map_err(|err| RadarError::new(format!("rank retrieve failed: {err}")))?;

    let mut hints = Vec::new();
    for neighbor in neighbors {
        let Some(chunk) = by_id.get(neighbor.id.as_str()) else {
            continue;
        };
        hints.push(RadarHint {
            chunk_id: chunk.chunk_id.clone(),
            title: chunk.title.clone(),
            span: chunk.span.clone(),
            capabilities: chunk.capabilities.clone(),
            preferred_codec: preferred_codec(chunk, query),
            confidence: confidence_from_score(neighbor.score)?,
        });
    }

    Ok(RadarReport {
        hints,
        stale_index: !stale_chunk_ids.is_empty(),
        stale_chunk_ids,
    })
}

fn embedding_vec(text: &str) -> Vec<f32> {
    let text = if text
        .split(|ch: char| !ch.is_alphanumeric())
        .any(|token| !token.is_empty())
    {
        text
    } else {
        DEFAULT_QUERY_TEXT
    };
    crate::embed(text).to_vec()
}

fn confidence_from_score(score: f32) -> RadarResult<f64> {
    let likelihood = ((score as f64 + 1.0) / 2.0).clamp(0.0, 1.0);
    let posterior = bayesian_update_binary(0.5, likelihood, 0.25)
        .map_err(|err| RadarError::new(format!("confidence update failed: {err}")))?;
    let uncertainty = entropy(&[posterior, 1.0 - posterior])
        .map_err(|err| RadarError::new(format!("confidence entropy failed: {err}")))?;
    Ok((posterior * 0.85 + (1.0 - uncertainty) * 0.15).clamp(0.0, 1.0))
}

fn preferred_codec(chunk: &RadarChunk, query: &RadarQuery) -> Option<String> {
    query
        .codec
        .clone()
        .or_else(|| chunk.codecs.first().cloned())
}

fn matches_field(value: &str, filter: &Option<String>) -> bool {
    filter
        .as_deref()
        .is_none_or(|filter| value.eq_ignore_ascii_case(filter))
}

fn matches_optional(value: Option<&str>, filter: &Option<String>) -> bool {
    filter
        .as_deref()
        .is_none_or(|filter| value.is_some_and(|value| value.eq_ignore_ascii_case(filter)))
}

fn matches_list(values: &[String], filter: &Option<String>) -> bool {
    filter.as_deref().is_none_or(|filter| {
        values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(filter))
    })
}

fn matches_list_or_text(values: &[String], text: &str, filter: &Option<String>) -> bool {
    filter.as_deref().is_none_or(|filter| {
        values
            .iter()
            .any(|value| value.eq_ignore_ascii_case(filter))
            || text
                .to_ascii_lowercase()
                .contains(&filter.to_ascii_lowercase())
    })
}
