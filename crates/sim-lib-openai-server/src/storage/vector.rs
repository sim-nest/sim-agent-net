use sim_kernel::Expr;

/// Record kind tag designating a gateway vector store.
pub const GATEWAY_VECTOR_STORE_KIND: &str = "openai-gateway/vector-store";
/// Record kind tag designating a single item within a gateway vector store.
pub const GATEWAY_VECTOR_STORE_ITEM_KIND: &str = "openai-gateway/vector-store-item";

/// A named gateway vector store holding embedded text items.
#[derive(Clone, Debug, PartialEq)]
pub struct GatewayVectorStore {
    id: String,
    name: String,
    created_at_ms: u64,
    items: Vec<GatewayVectorStoreItem>,
}

impl GatewayVectorStore {
    /// Creates a vector store with the given id, name, creation time, and items.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        created_at_ms: u64,
        items: Vec<GatewayVectorStoreItem>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            created_at_ms,
            items,
        }
    }

    /// Returns the vector store's identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the vector store's display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the creation timestamp in milliseconds since the Unix epoch.
    pub fn created_at_ms(&self) -> u64 {
        self.created_at_ms
    }

    /// Returns the items held by this vector store.
    pub fn items(&self) -> &[GatewayVectorStoreItem] {
        &self.items
    }

    /// Encodes the vector store as a SIM [`Expr`] map record.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field("kind", Expr::String(GATEWAY_VECTOR_STORE_KIND.to_owned())),
            field("id", Expr::String(self.id.clone())),
            field("name", Expr::String(self.name.clone())),
            field(
                "created-at-ms",
                Expr::String(self.created_at_ms.to_string()),
            ),
            field(
                "items",
                Expr::Vector(
                    self.items
                        .iter()
                        .map(GatewayVectorStoreItem::to_expr)
                        .collect(),
                ),
            ),
        ])
    }
}

/// A single embedded text item within a [`GatewayVectorStore`].
#[derive(Clone, Debug, PartialEq)]
pub struct GatewayVectorStoreItem {
    id: String,
    text: String,
    embedding: Vec<f64>,
}

impl GatewayVectorStoreItem {
    /// Creates an item with the given id, text, and embedding vector.
    pub fn new(id: impl Into<String>, text: impl Into<String>, embedding: Vec<f64>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            embedding,
        }
    }

    /// Returns the item's identifier.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the item's source text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the item's embedding vector.
    pub fn embedding(&self) -> &[f64] {
        &self.embedding
    }

    /// Encodes the item as a SIM [`Expr`] map record.
    pub fn to_expr(&self) -> Expr {
        Expr::Map(vec![
            field(
                "kind",
                Expr::String(GATEWAY_VECTOR_STORE_ITEM_KIND.to_owned()),
            ),
            field("id", Expr::String(self.id.clone())),
            field("text", Expr::String(self.text.clone())),
            field(
                "embedding",
                Expr::Vector(
                    self.embedding
                        .iter()
                        .map(|value| Expr::String(format!("{value:.6}")))
                        .collect(),
                ),
            ),
        ])
    }
}

use sim_value::build::entry as field;
