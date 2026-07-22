use sim_kernel::{CapabilityName, Expr, Result, ShapeRef, Symbol};

use crate::surface::{public_annotations, stable_mcp_name_text};
use crate::{
    McpAnnotation, McpStreamPolicy, McpSurfaceCard, McpSurfaceRole, McpSurfaceSource,
    stable_mcp_name,
};

/// Native browse card describing one runtime subject and its MCP facets.
#[derive(Clone)]
pub struct McpNativeCard {
    /// Runtime symbol the card describes.
    pub subject: Symbol,
    /// Human-readable description.
    pub description: String,
    /// Default input shape applied to exported facets.
    pub input_shape: Option<ShapeRef>,
    /// Default output shape applied to exported facets.
    pub output_shape: Option<ShapeRef>,
    /// Capabilities required to invoke the subject.
    pub capabilities: Vec<CapabilityName>,
    /// Facets attached to the card, including MCP exports.
    pub facets: Vec<NativeFacet>,
}

impl McpNativeCard {
    /// Creates a card for `subject` with the given `description`.
    pub fn new(subject: Symbol, description: impl Into<String>) -> Self {
        Self {
            subject,
            description: description.into(),
            input_shape: None,
            output_shape: None,
            capabilities: Vec::new(),
            facets: Vec::new(),
        }
    }

    /// Returns the card with default input and output shapes set.
    pub fn with_shapes(mut self, input: ShapeRef, output: ShapeRef) -> Self {
        self.input_shape = Some(input);
        self.output_shape = Some(output);
        self
    }

    /// Returns the card with `capability` added to its requirements.
    pub fn with_capability(mut self, capability: CapabilityName) -> Self {
        self.capabilities.push(capability);
        self
    }

    /// Returns the card with `facet` appended.
    pub fn with_facet(mut self, facet: NativeFacet) -> Self {
        self.facets.push(facet);
        self
    }

    /// Returns the card with `export` appended as an MCP export facet.
    pub fn exported(mut self, export: McpExportFacet) -> Self {
        self.facets.push(NativeFacet::McpExport(export));
        self
    }
}

/// A facet attached to an [`McpNativeCard`].
#[derive(Clone)]
pub enum NativeFacet {
    /// An MCP export that projects onto the surface.
    McpExport(McpExportFacet),
    /// An arbitrary named facet carried on the card.
    Other {
        /// Facet name.
        name: Symbol,
        /// Facet value.
        value: Expr,
        /// Whether the facet is internal-only.
        private: bool,
    },
}

/// Describes how a native card subject is exported onto the MCP surface.
#[derive(Clone)]
pub struct McpExportFacet {
    /// MCP role of the exported row.
    pub role: McpSurfaceRole,
    /// Explicit MCP name; defaults to the subject when absent.
    pub name: Option<String>,
    /// Backing symbol; defaults to the card subject when absent.
    pub symbol: Option<Symbol>,
    /// Resource URI override.
    pub uri: Option<String>,
    /// Description override.
    pub description: Option<String>,
    /// Input shape override.
    pub input_shape: Option<ShapeRef>,
    /// Output shape override.
    pub output_shape: Option<ShapeRef>,
    /// Annotations attached to the exported row.
    pub annotations: Vec<McpAnnotation>,
    /// Extra capabilities required by the exported row.
    pub capabilities: Vec<CapabilityName>,
    /// Streaming behavior advertised by the exported row.
    pub stream_policy: McpStreamPolicy,
}

impl McpExportFacet {
    /// Builds a tool export facet.
    pub fn tool() -> Self {
        Self::new(McpSurfaceRole::Tool)
    }

    /// Builds a resource export facet.
    pub fn resource() -> Self {
        Self::new(McpSurfaceRole::Resource)
    }

    /// Builds a prompt export facet.
    pub fn prompt() -> Self {
        Self::new(McpSurfaceRole::Prompt)
    }

    /// Builds a model export facet.
    pub fn model() -> Self {
        Self::new(McpSurfaceRole::Model)
    }

    /// Builds an export facet for `role` with all overrides unset.
    pub fn new(role: McpSurfaceRole) -> Self {
        Self {
            role,
            name: None,
            symbol: None,
            uri: None,
            description: None,
            input_shape: None,
            output_shape: None,
            annotations: Vec::new(),
            capabilities: Vec::new(),
            stream_policy: McpStreamPolicy::None,
        }
    }

    /// Returns the facet with an explicit MCP `name`.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Returns the facet with an explicit backing `symbol`.
    pub fn with_symbol(mut self, symbol: Symbol) -> Self {
        self.symbol = Some(symbol);
        self
    }

    /// Returns the facet with an explicit resource `uri`.
    pub fn with_uri(mut self, uri: impl Into<String>) -> Self {
        self.uri = Some(uri.into());
        self
    }

    /// Returns the facet with an explicit `description`.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Returns the facet with `annotation` appended.
    pub fn with_annotation(mut self, annotation: McpAnnotation) -> Self {
        self.annotations.push(annotation);
        self
    }

    /// Returns the facet with `capability` added to its requirements.
    pub fn with_capability(mut self, capability: CapabilityName) -> Self {
        self.capabilities.push(capability);
        self
    }

    /// Returns the facet with its stream `policy` set.
    pub fn with_stream_policy(mut self, policy: McpStreamPolicy) -> Self {
        self.stream_policy = policy;
        self
    }
}

/// Returns the facet name that marks MCP exports on native cards.
pub fn mcp_export_facet_name() -> Symbol {
    Symbol::new("mcp-export")
}

/// Returns the operation symbol identifying the MCP export operation.
pub fn mcp_export_operation_symbol() -> Symbol {
    Symbol::qualified("mcp", "export")
}

/// Projects the MCP export facets of `cards` into surface rows.
pub fn native_surface_rows(cards: &[McpNativeCard]) -> Result<Vec<McpSurfaceCard>> {
    let mut rows = Vec::new();
    for card in cards {
        for facet in &card.facets {
            if let NativeFacet::McpExport(export) = facet {
                rows.push(row_from_export(card, export)?);
            }
        }
    }
    Ok(rows)
}

fn row_from_export(card: &McpNativeCard, export: &McpExportFacet) -> Result<McpSurfaceCard> {
    let symbol = export
        .symbol
        .clone()
        .unwrap_or_else(|| card.subject.clone());
    let name = match &export.name {
        Some(name) => stable_mcp_name_text(name)?,
        None => stable_mcp_name(&symbol)?,
    };
    let mut capabilities = card.capabilities.clone();
    capabilities.extend(export.capabilities.clone());
    capabilities.sort();
    capabilities.dedup();

    Ok(McpSurfaceCard {
        id: format!("native:{}:{}", export.role.as_symbol(), card.subject),
        source: McpSurfaceSource::NativeCard,
        role: export.role.clone(),
        name,
        symbol: Some(symbol),
        uri: Some(
            export
                .uri
                .clone()
                .unwrap_or_else(|| format!("sim://{}", card.subject)),
        ),
        description: export
            .description
            .clone()
            .unwrap_or_else(|| card.description.clone()),
        input_shape: export
            .input_shape
            .clone()
            .or_else(|| card.input_shape.clone()),
        output_shape: export
            .output_shape
            .clone()
            .or_else(|| card.output_shape.clone()),
        annotations: public_annotations(&export.annotations),
        capabilities,
        stream_policy: export.stream_policy.clone(),
    })
}
