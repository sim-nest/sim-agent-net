use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_skill::{SkillCard, SkillEventSink, SkillTransport};

use super::{field, optional_field};
use crate::client::peer::{McpClientPeer, request_peer};

/// Skill transport that dispatches imported skill operations back to an MCP
/// peer via `tools/call`, `resources/read`, and `prompts/get`.
pub struct McpClientTransport {
    id: String,
    peer: Arc<dyn McpClientPeer>,
    operations: Mutex<BTreeMap<String, ForeignOperation>>,
    next_id: AtomicU64,
}

impl McpClientTransport {
    /// Creates a transport identified by `id` that dispatches through `peer`.
    pub fn new(id: impl Into<String>, peer: Arc<dyn McpClientPeer>) -> Self {
        Self {
            id: id.into(),
            peer,
            operations: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    pub(crate) fn insert(&self, operation: String, foreign: ForeignOperation) -> Result<()> {
        self.operations
            .lock()
            .map_err(|_| Error::PoisonedLock("mcp client operations"))?
            .insert(operation, foreign);
        Ok(())
    }

    fn request(&self, cx: &mut Cx, method: &str, params: Expr) -> Result<Expr> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        request_peer(
            cx,
            self.peer.as_ref(),
            Expr::String(format!("{}-call-{id}", self.id)),
            method,
            params,
        )
    }
}

impl SkillTransport for McpClientTransport {
    fn id(&self) -> &str {
        &self.id
    }

    fn kind(&self) -> &str {
        "mcp-client"
    }

    fn discover(&self, _cx: &mut Cx) -> Result<Vec<SkillCard>> {
        Ok(Vec::new())
    }

    fn call(
        &self,
        cx: &mut Cx,
        card: &SkillCard,
        args: Value,
        _events: Option<&mut dyn SkillEventSink>,
    ) -> Result<Value> {
        let operation = self
            .operations
            .lock()
            .map_err(|_| Error::PoisonedLock("mcp client operations"))?
            .get(&card.operation)
            .cloned()
            .ok_or_else(|| Error::Eval(format!("missing foreign MCP operation {}", card.id)))?;
        match operation {
            ForeignOperation::Tool(name) => {
                let params = tool_call_params(cx, &name, args)?;
                let result = self.request(cx, "tools/call", params)?;
                value_from_mcp_content_result(cx, &result, "content")
            }
            ForeignOperation::Resource(uri) => {
                let result = self.request(cx, "resources/read", resource_read_params(&uri))?;
                value_from_mcp_content_result(cx, &result, "contents")
            }
            ForeignOperation::Prompt(name) => {
                let params = prompt_get_params(cx, &name, args)?;
                let result = self.request(cx, "prompts/get", params)?;
                value_from_mcp_prompt_result(cx, &result)
            }
        }
    }

    fn health(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("mcp-client"))?,
            ),
            (Symbol::new("id"), cx.factory().string(self.id.clone())?),
        ])
    }
}

#[derive(Clone)]
pub(crate) enum ForeignOperation {
    Tool(String),
    Resource(String),
    Prompt(String),
}

fn tool_call_params(cx: &mut Cx, name: &str, args: Value) -> Result<Expr> {
    Ok(Expr::Map(vec![
        field("name", Expr::String(name.to_owned())),
        field("arguments", args.object().as_expr(cx)?),
    ]))
}

fn resource_read_params(uri: &str) -> Expr {
    Expr::Map(vec![field("uri", Expr::String(uri.to_owned()))])
}

fn prompt_get_params(cx: &mut Cx, name: &str, args: Value) -> Result<Expr> {
    let arguments = match args.object().as_expr(cx)? {
        Expr::List(items) if items.is_empty() => Expr::Nil,
        Expr::List(mut items) if items.len() == 1 => items.remove(0),
        Expr::List(items) => Expr::List(items),
        other => other,
    };
    Ok(Expr::Map(vec![
        field("name", Expr::String(name.to_owned())),
        field("arguments", arguments),
    ]))
}

fn value_from_mcp_content_result(cx: &mut Cx, result: &Expr, field_name: &str) -> Result<Value> {
    if matches!(optional_bool(result, "isError"), Some(true)) {
        return Err(Error::Eval(
            content_text(result, field_name)
                .unwrap_or_else(|| "foreign MCP operation returned an error".to_owned()),
        ));
    }
    let content = single_content(result, field_name)?;
    value_from_content(cx, content)
}

fn value_from_mcp_prompt_result(cx: &mut Cx, result: &Expr) -> Result<Value> {
    if matches!(optional_bool(result, "isError"), Some(true)) {
        return Err(Error::Eval(
            content_text(result, "messages")
                .unwrap_or_else(|| "foreign MCP prompt returned an error".to_owned()),
        ));
    }
    match optional_field(result, "messages") {
        Some(Expr::List(messages)) if messages.len() == 1 => {
            value_from_prompt_message(cx, &messages[0])
        }
        Some(Expr::List(messages)) => cx.factory().expr(Expr::List(messages.clone())),
        _ => cx.factory().expr(result.clone()),
    }
}

fn value_from_prompt_message(cx: &mut Cx, message: &Expr) -> Result<Value> {
    let Some(content) = optional_field(message, "content") else {
        return cx.factory().expr(message.clone());
    };
    value_from_content(cx, content)
}

fn value_from_content(cx: &mut Cx, content: &Expr) -> Result<Value> {
    match optional_field(content, "json") {
        Some(expr) => cx.factory().expr(expr.clone()),
        None => match optional_field(content, "text") {
            Some(Expr::String(text)) => cx.factory().string(text.clone()),
            _ => cx.factory().expr(content.clone()),
        },
    }
}

fn single_content<'a>(result: &'a Expr, field_name: &str) -> Result<&'a Expr> {
    match optional_field(result, field_name) {
        Some(Expr::List(items)) if items.len() == 1 => Ok(&items[0]),
        Some(Expr::List(items)) if !items.is_empty() => Ok(&items[0]),
        _ => Err(Error::Eval(format!(
            "foreign MCP result missing {field_name} content"
        ))),
    }
}

fn content_text(result: &Expr, field_name: &str) -> Option<String> {
    single_content(result, field_name)
        .ok()
        .and_then(|content| optional_field(content, "text"))
        .and_then(|text| match text {
            Expr::String(text) => Some(text.clone()),
            _ => None,
        })
}

fn optional_bool(expr: &Expr, name: &str) -> Option<bool> {
    match optional_field(expr, name) {
        Some(Expr::Bool(value)) => Some(*value),
        _ => None,
    }
}
