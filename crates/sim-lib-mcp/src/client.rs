mod descriptors;
mod peer;
mod transport;

use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use sim_kernel::{CapabilityName, Cx, Error, Expr, Result};
use sim_lib_skill::{SkillCard, SkillTransport};

use descriptors::{ForeignPromptDescriptor, ForeignResourceDescriptor, ForeignToolDescriptor};
use peer::request_peer;
pub use peer::{McpClientCassettePeer, McpClientPeer, RouterMcpPeer};
use transport::ForeignOperation;
pub use transport::McpClientTransport;

/// Returns the capability gating outbound MCP client requests.
pub fn mcp_client_capability() -> CapabilityName {
    CapabilityName::new("mcp.client")
}

/// Outbound MCP client that imports a peer server's tools, resources, and
/// prompts as SIM skill cards.
pub struct McpClient {
    id: String,
    peer: Arc<dyn McpClientPeer>,
    next_id: AtomicU64,
}

impl McpClient {
    /// Creates a client identified by `id` that talks to `peer`.
    pub fn new(id: impl Into<String>, peer: Arc<dyn McpClientPeer>) -> Self {
        Self {
            id: id.into(),
            peer,
            next_id: AtomicU64::new(1),
        }
    }

    /// Initializes the peer, lists its tools/resources/prompts, and imports the
    /// ones allowed by `policy` as skill cards bound into the runtime.
    pub fn import_allowed(
        &self,
        cx: &mut Cx,
        policy: &McpClientAllowPolicy,
    ) -> Result<Vec<SkillCard>> {
        cx.require(&mcp_client_capability())?;
        sim_lib_skill::install_skill_lib(cx)?;
        self.request(cx, "initialize", initialize_params(&self.id))?;
        let tools = self.list_tools(cx)?;
        let resources = self.list_resources(cx)?;
        let prompts = self.list_prompts(cx)?;
        let transport = Arc::new(McpClientTransport::new(self.id.clone(), self.peer.clone()));
        let registry = sim_lib_skill::skill_registry(cx)?;
        registry.install_transport(transport.clone())?;
        let mut cards = Vec::new();
        for tool in tools
            .into_iter()
            .filter(|tool| policy.allows_tool(&tool.name))
        {
            let card = tool.to_skill_card(&self.id, transport.id())?;
            transport.insert(card.operation.clone(), ForeignOperation::Tool(tool.name))?;
            registry.bind_card(cx, card.clone())?;
            cards.push(card);
        }
        for resource in resources
            .into_iter()
            .filter(|resource| policy.allows_resource(&resource.uri))
        {
            let card = resource.to_skill_card(&self.id, transport.id())?;
            transport.insert(
                card.operation.clone(),
                ForeignOperation::Resource(resource.uri),
            )?;
            registry.bind_card(cx, card.clone())?;
            cards.push(card);
        }
        for prompt in prompts
            .into_iter()
            .filter(|prompt| policy.allows_prompt(&prompt.name))
        {
            let card = prompt.to_skill_card(&self.id, transport.id())?;
            transport.insert(
                card.operation.clone(),
                ForeignOperation::Prompt(prompt.name),
            )?;
            registry.bind_card(cx, card.clone())?;
            cards.push(card);
        }
        Ok(cards)
    }

    fn list_tools(&self, cx: &mut Cx) -> Result<Vec<ForeignToolDescriptor>> {
        let result = self.request(cx, "tools/list", Expr::Nil)?;
        list_field(&result, "tools")?
            .iter()
            .map(ForeignToolDescriptor::from_expr)
            .collect()
    }

    fn list_resources(&self, cx: &mut Cx) -> Result<Vec<ForeignResourceDescriptor>> {
        let result = self.request(cx, "resources/list", Expr::Nil)?;
        list_field(&result, "resources")?
            .iter()
            .map(ForeignResourceDescriptor::from_expr)
            .collect()
    }

    fn list_prompts(&self, cx: &mut Cx) -> Result<Vec<ForeignPromptDescriptor>> {
        let result = self.request(cx, "prompts/list", Expr::Nil)?;
        list_field(&result, "prompts")?
            .iter()
            .map(ForeignPromptDescriptor::from_expr)
            .collect()
    }

    fn request(&self, cx: &mut Cx, method: &str, params: Expr) -> Result<Expr> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        request_peer(
            cx,
            self.peer.as_ref(),
            Expr::String(format!("{}-{id}", self.id)),
            method,
            params,
        )
    }
}

/// Allow-list controlling which peer tools, resources, and prompts are imported.
#[derive(Clone, Default)]
pub struct McpClientAllowPolicy {
    tools: BTreeSet<String>,
    resources: BTreeSet<String>,
    prompts: BTreeSet<String>,
}

impl McpClientAllowPolicy {
    /// Creates an empty policy that allows nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the policy with tool `name` allowed.
    pub fn allow_tool(mut self, name: impl Into<String>) -> Self {
        self.tools.insert(name.into());
        self
    }

    /// Returns the policy with resource `uri` allowed.
    pub fn allow_resource(mut self, uri: impl Into<String>) -> Self {
        self.resources.insert(uri.into());
        self
    }

    /// Returns the policy with prompt `name` allowed.
    pub fn allow_prompt(mut self, name: impl Into<String>) -> Self {
        self.prompts.insert(name.into());
        self
    }

    fn allows_tool(&self, name: &str) -> bool {
        self.tools.contains(name)
    }

    fn allows_resource(&self, uri: &str) -> bool {
        self.resources.contains(uri)
    }

    fn allows_prompt(&self, name: &str) -> bool {
        self.prompts.contains(name)
    }
}

fn initialize_params(client_id: &str) -> Expr {
    Expr::Map(vec![
        field(
            "protocolVersion",
            Expr::String(crate::session::DEFAULT_PROTOCOL_VERSION.to_owned()),
        ),
        field(
            "clientInfo",
            Expr::Map(vec![
                field("name", Expr::String(client_id.to_owned())),
                field(
                    "version",
                    Expr::String(env!("CARGO_PKG_VERSION").to_owned()),
                ),
            ]),
        ),
    ])
}

fn list_field<'a>(expr: &'a Expr, name: &str) -> Result<&'a [Expr]> {
    match optional_field(expr, name) {
        Some(Expr::List(items)) => Ok(items),
        _ => Err(Error::TypeMismatch {
            expected: "MCP list field",
            found: "missing or non-list",
        }),
    }
}

pub(crate) use sim_value::access::{
    entry_field_any as optional_field_from_fields, field_any as optional_field,
};
pub(crate) use sim_value::build::entry as field;

#[cfg(test)]
mod tests {
    use super::*;
    use sim_kernel::Symbol;

    #[test]
    fn optional_fields_use_provider_key_policy() {
        let map = Expr::Map(vec![
            field("bare", Expr::String("bare".to_owned())),
            (
                Expr::String("string".to_owned()),
                Expr::String("string".to_owned()),
            ),
            (
                Expr::Symbol(Symbol::qualified("mcp", "qualified")),
                Expr::String("qualified".to_owned()),
            ),
        ]);
        let Expr::Map(entries) = &map else {
            panic!("test map");
        };

        assert!(matches!(
            optional_field(&map, "bare"),
            Some(Expr::String(value)) if value == "bare"
        ));
        assert!(matches!(
            optional_field_from_fields(entries, "string"),
            Some(Expr::String(value)) if value == "string"
        ));
        assert_eq!(optional_field(&map, "qualified"), None);
    }
}
