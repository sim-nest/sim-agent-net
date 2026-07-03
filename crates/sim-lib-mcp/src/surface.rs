use std::collections::BTreeMap;

use sim_kernel::{CapabilityName, Cx, Error, Expr, Result, ShapeRef, Symbol};

use crate::{McpNativeCard, McpProfile, native_surface_rows};

/// Origin record that a projected [`McpSurfaceCard`] was derived from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpSurfaceSource {
    /// The row was projected from a native browse [`McpNativeCard`].
    NativeCard,
    /// The row was projected from an agent skill card.
    SkillCard,
}

impl McpSurfaceSource {
    /// Returns the stable wire symbol naming this source.
    pub fn as_symbol(&self) -> Symbol {
        Symbol::new(match self {
            Self::NativeCard => "native-card",
            Self::SkillCard => "skill-card",
        })
    }
}

/// MCP role that a projected surface row plays for a client.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum McpSurfaceRole {
    /// A callable tool.
    Tool,
    /// A readable resource.
    Resource,
    /// A prompt template.
    Prompt,
    /// A sampling model surface.
    Model,
}

impl McpSurfaceRole {
    /// Returns the stable wire symbol naming this role.
    pub fn as_symbol(&self) -> Symbol {
        Symbol::new(match self {
            Self::Tool => "tool",
            Self::Resource => "resource",
            Self::Prompt => "prompt",
            Self::Model => "model",
        })
    }
}

/// Streaming behavior a surface row advertises to clients.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum McpStreamPolicy {
    /// No streaming; a single response is returned.
    #[default]
    None,
    /// The call emits incremental progress notifications.
    Progress,
    /// The call emits a stream of data chunks.
    DataStream,
}

impl McpStreamPolicy {
    /// Returns the stable wire symbol naming this stream policy.
    pub fn as_symbol(&self) -> Symbol {
        Symbol::new(match self {
            Self::None => "none",
            Self::Progress => "progress",
            Self::DataStream => "data-stream",
        })
    }
}

/// Whether an [`McpAnnotation`] is exposed to clients or kept internal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpAnnotationVisibility {
    /// The annotation is projected onto the public surface.
    Public,
    /// The annotation is retained internally and never surfaced.
    Private,
}

/// Key/value annotation attached to a surface row, with a visibility scope.
#[derive(Clone, Debug, PartialEq)]
pub struct McpAnnotation {
    /// Annotation key.
    pub key: Symbol,
    /// Annotation value expression.
    pub value: Expr,
    /// Whether the annotation is surfaced to clients.
    pub visibility: McpAnnotationVisibility,
}

impl McpAnnotation {
    /// Builds a [`McpAnnotationVisibility::Public`] annotation.
    pub fn public(key: impl Into<Symbol>, value: Expr) -> Self {
        Self {
            key: key.into(),
            value,
            visibility: McpAnnotationVisibility::Public,
        }
    }

    /// Builds a [`McpAnnotationVisibility::Private`] annotation.
    pub fn private(key: impl Into<Symbol>, value: Expr) -> Self {
        Self {
            key: key.into(),
            value,
            visibility: McpAnnotationVisibility::Private,
        }
    }
}

/// A single redacted MCP surface row projected from a native or skill card.
#[derive(Clone)]
pub struct McpSurfaceCard {
    /// Stable identifier for the row.
    pub id: String,
    /// Card the row was projected from.
    pub source: McpSurfaceSource,
    /// MCP role the row plays.
    pub role: McpSurfaceRole,
    /// Public MCP name.
    pub name: String,
    /// Backing runtime symbol, when the row maps to one.
    pub symbol: Option<Symbol>,
    /// Resource URI, for resource rows.
    pub uri: Option<String>,
    /// Human-readable description.
    pub description: String,
    /// Input shape, when the row accepts arguments.
    pub input_shape: Option<ShapeRef>,
    /// Output shape, when the row declares a result shape.
    pub output_shape: Option<ShapeRef>,
    /// Public annotations carried onto the surface.
    pub annotations: Vec<(Symbol, Expr)>,
    /// Capabilities required to invoke the row.
    pub capabilities: Vec<CapabilityName>,
    /// Streaming behavior advertised by the row.
    pub stream_policy: McpStreamPolicy,
}

impl McpSurfaceCard {
    /// Encodes the row as an `mcp/surface-card` [`Expr`] map.
    pub fn to_expr(&self, cx: &mut Cx) -> Result<Expr> {
        Ok(Expr::Map(vec![
            field(
                "kind",
                Expr::Symbol(Symbol::qualified("mcp", "surface-card")),
            ),
            field("id", Expr::String(self.id.clone())),
            field("source", Expr::Symbol(self.source.as_symbol())),
            field("role", Expr::Symbol(self.role.as_symbol())),
            field("name", Expr::String(self.name.clone())),
            field(
                "symbol",
                self.symbol.clone().map(Expr::Symbol).unwrap_or(Expr::Nil),
            ),
            field(
                "uri",
                self.uri
                    .as_ref()
                    .map(|uri| Expr::String(uri.clone()))
                    .unwrap_or(Expr::Nil),
            ),
            field("description", Expr::String(self.description.clone())),
            field("input-shape", shape_expr(cx, &self.input_shape)?),
            field("output-shape", shape_expr(cx, &self.output_shape)?),
            field(
                "annotations",
                Expr::List(
                    self.annotations
                        .iter()
                        .map(|(key, value)| {
                            Expr::Map(vec![
                                field("key", Expr::Symbol(key.clone())),
                                field("value", value.clone()),
                            ])
                        })
                        .collect(),
                ),
            ),
            field(
                "capabilities",
                Expr::List(
                    self.capabilities
                        .iter()
                        .map(|capability| Expr::String(capability.as_str().to_owned()))
                        .collect(),
                ),
            ),
            field(
                "stream-policy",
                Expr::Symbol(self.stream_policy.as_symbol()),
            ),
        ]))
    }
}

/// Projects native cards into surface rows filtered by `profile`.
pub fn project_native_surface(
    native_cards: &[McpNativeCard],
    profile: &McpProfile,
) -> Result<Vec<McpSurfaceCard>> {
    project_surface_rows(native_surface_rows(native_cards)?, profile)
}

/// Projects native cards plus (when the `skill` feature is on) skill cards into
/// surface rows filtered by `profile`.
pub fn project_mcp_surface(
    cx: &mut Cx,
    native_cards: &[McpNativeCard],
    profile: &McpProfile,
) -> Result<Vec<McpSurfaceCard>> {
    #[cfg(not(feature = "skill"))]
    let _ = cx;

    let rows = native_surface_rows(native_cards)?;
    #[cfg(feature = "skill")]
    let rows = {
        let mut rows = rows;
        rows.extend(crate::skill::skill_surface_rows(cx)?);
        rows
    };
    project_surface_rows(rows, profile)
}

/// Filters `rows` through `profile` and deduplicates by name, returning the
/// rows sorted by name; errors on a name collision among allowed rows.
pub fn project_surface_rows(
    rows: Vec<McpSurfaceCard>,
    profile: &McpProfile,
) -> Result<Vec<McpSurfaceCard>> {
    let mut by_name = BTreeMap::new();
    for row in rows.into_iter().filter(|row| profile.allows(row)) {
        if let Some(existing) = by_name.insert(row.name.clone(), row) {
            return Err(Error::Eval(format!(
                "MCP surface name collision for {}",
                existing.name
            )));
        }
    }
    Ok(by_name.into_values().collect())
}

/// Derives a stable, ASCII MCP name from a runtime `symbol`.
pub fn stable_mcp_name(symbol: &Symbol) -> Result<String> {
    stable_mcp_name_text(&symbol.to_string())
}

pub(crate) fn stable_mcp_name_text(text: &str) -> Result<String> {
    if !text.is_ascii() {
        return Err(Error::Eval(format!("MCP name {text} is not ASCII")));
    }
    let mut output = String::new();
    let mut last_was_separator = false;
    for ch in text.chars() {
        let mapped = match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' => ch,
            '/' | '.' => '.',
            _ => '-',
        };
        if mapped == '.' || mapped == '-' {
            if last_was_separator {
                continue;
            }
            last_was_separator = true;
        } else {
            last_was_separator = false;
        }
        output.push(mapped);
    }
    let output = output.trim_matches(['.', '-']).to_owned();
    if output.is_empty() {
        return Err(Error::Eval("MCP name cannot be empty".to_owned()));
    }
    Ok(output)
}

pub(crate) fn public_annotations(annotations: &[McpAnnotation]) -> Vec<(Symbol, Expr)> {
    annotations
        .iter()
        .filter(|annotation| annotation.visibility == McpAnnotationVisibility::Public)
        .map(|annotation| (annotation.key.clone(), annotation.value.clone()))
        .collect()
}

fn shape_expr(cx: &mut Cx, shape: &Option<ShapeRef>) -> Result<Expr> {
    match shape {
        Some(shape) => shape.object().as_expr(cx),
        None => Ok(Expr::Nil),
    }
}

use sim_value::build::entry as field;
