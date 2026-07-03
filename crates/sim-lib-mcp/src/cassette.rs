use sim_citizen::CitizenField;
use sim_citizen_derive::Citizen;
use sim_codec_mcp::{McpEnvelope, envelope_to_expr, expr_to_envelope};
use sim_kernel::{Expr, Result, Symbol};

/// One recorded request and its replies in an [`McpCassette`].
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "mcp/CassetteEntry", version = 1)]
pub struct McpCassetteEntry {
    /// Canonical, redacted request envelope.
    pub request: Expr,
    /// Canonical, redacted reply envelopes.
    pub replies: Vec<Expr>,
}

/// One audited MCP operation outcome recorded in an [`McpCassette`].
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "mcp/AuditEntry", version = 1)]
pub struct McpAuditEntry {
    /// MCP method name.
    pub method: String,
    /// Logical operation the method maps to.
    pub operation: String,
    /// Recorded outcome (for example `ok`, `error`, or a denial reason).
    pub outcome: String,
}

/// Record/replay log of redacted MCP exchanges plus an audit trail.
#[derive(Clone, Debug, Default, PartialEq, Citizen)]
#[citizen(symbol = "mcp/Cassette", version = 1)]
pub struct McpCassette {
    entries: Vec<McpCassetteEntry>,
    replay_index: usize,
    audit: Vec<McpAuditEntry>,
}

impl Default for McpCassetteEntry {
    fn default() -> Self {
        Self {
            request: envelope_to_expr(&McpEnvelope::Request(sim_codec_mcp::McpRequest::default())),
            replies: vec![envelope_to_expr(&McpEnvelope::Response(
                sim_codec_mcp::McpResponse::default(),
            ))],
        }
    }
}

impl Default for McpAuditEntry {
    fn default() -> Self {
        Self {
            method: "tools/list".to_owned(),
            operation: "list".to_owned(),
            outcome: "ok".to_owned(),
        }
    }
}

impl CitizenField for McpCassetteEntry {
    fn encode_field(&self) -> Expr {
        Expr::List(vec![
            self.request.encode_field(),
            self.replies.encode_field(),
        ])
    }

    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        let Expr::List(items) = expr else {
            return Err(sim_citizen::field_error(
                field,
                "expected MCP cassette entry list",
            ));
        };
        let [request, replies] = items.as_slice() else {
            return Err(sim_citizen::field_error(
                field,
                format!(
                    "expected 2 MCP cassette entry field(s), found {}",
                    items.len()
                ),
            ));
        };
        Ok(Self {
            request: Expr::decode_field_expr(request, field)?,
            replies: Vec::<Expr>::decode_field_expr(replies, field)?,
        })
    }
}

impl CitizenField for McpAuditEntry {
    fn encode_field(&self) -> Expr {
        Expr::List(vec![
            self.method.encode_field(),
            self.operation.encode_field(),
            self.outcome.encode_field(),
        ])
    }

    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        let Expr::List(items) = expr else {
            return Err(sim_citizen::field_error(
                field,
                "expected MCP audit entry list",
            ));
        };
        let [method, operation, outcome] = items.as_slice() else {
            return Err(sim_citizen::field_error(
                field,
                format!("expected 3 MCP audit entry field(s), found {}", items.len()),
            ));
        };
        Ok(Self {
            method: String::decode_field_expr(method, field)?,
            operation: String::decode_field_expr(operation, field)?,
            outcome: String::decode_field_expr(outcome, field)?,
        })
    }
}

impl McpCassette {
    /// Creates an empty cassette.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a cassette preloaded with `entries` for replay.
    pub fn from_entries(entries: Vec<McpCassetteEntry>) -> Self {
        Self {
            entries,
            replay_index: 0,
            audit: Vec::new(),
        }
    }

    /// Returns the recorded exchange entries.
    pub fn entries(&self) -> &[McpCassetteEntry] {
        &self.entries
    }

    /// Returns the recorded audit entries.
    pub fn audit(&self) -> &[McpAuditEntry] {
        &self.audit
    }

    /// Records a redacted `request`/`replies` exchange.
    pub fn record_exchange(
        &mut self,
        request: &McpEnvelope,
        replies: &[McpEnvelope],
    ) -> Result<()> {
        self.entries.push(McpCassetteEntry {
            request: canonical_envelope(request)?,
            replies: replies
                .iter()
                .map(canonical_envelope)
                .collect::<Result<Vec<_>>>()?,
        });
        Ok(())
    }

    /// Replays the next recorded reply set when `request` matches it in order;
    /// returns `None` on a mismatch or when the log is exhausted.
    pub fn replay(&mut self, request: &McpEnvelope) -> Result<Option<Vec<McpEnvelope>>> {
        let request = canonical_envelope(request)?;
        let Some(entry) = self.entries.get(self.replay_index) else {
            return Ok(None);
        };
        if entry.request != request {
            return Ok(None);
        }
        self.replay_index += 1;
        entry
            .replies
            .iter()
            .map(expr_to_envelope)
            .collect::<Result<Vec<_>>>()
            .map(Some)
    }

    /// Appends an audit entry for `method`/`operation` with `outcome`.
    pub fn record_audit(
        &mut self,
        method: impl Into<String>,
        operation: impl Into<String>,
        outcome: impl Into<String>,
    ) {
        self.audit.push(McpAuditEntry {
            method: method.into(),
            operation: operation.into(),
            outcome: outcome.into(),
        });
    }
}

fn canonical_envelope(envelope: &McpEnvelope) -> Result<Expr> {
    let expr = redact_expr(envelope_to_expr(envelope), false);
    let envelope = expr_to_envelope(&expr)?;
    Ok(envelope_to_expr(&envelope))
}

fn redact_expr(expr: Expr, sensitive: bool) -> Expr {
    if sensitive {
        return Expr::String("[redacted]".to_owned());
    }
    match expr {
        Expr::String(text) if looks_secret(&text) => Expr::String("[redacted]".to_owned()),
        Expr::List(items) => Expr::List(
            items
                .into_iter()
                .map(|item| redact_expr(item, false))
                .collect(),
        ),
        Expr::Vector(items) => Expr::Vector(
            items
                .into_iter()
                .map(|item| redact_expr(item, false))
                .collect(),
        ),
        Expr::Set(items) => Expr::Set(
            items
                .into_iter()
                .map(|item| redact_expr(item, false))
                .collect(),
        ),
        Expr::Map(fields) => Expr::Map(
            fields
                .into_iter()
                .map(|(key, value)| {
                    let sensitive = sensitive_key(&key);
                    (redact_expr(key, false), redact_expr(value, sensitive))
                })
                .collect(),
        ),
        Expr::Call { operator, args } => Expr::Call {
            operator: Box::new(redact_expr(*operator, false)),
            args: args
                .into_iter()
                .map(|arg| redact_expr(arg, false))
                .collect(),
        },
        Expr::Infix {
            operator,
            left,
            right,
        } => Expr::Infix {
            operator,
            left: Box::new(redact_expr(*left, false)),
            right: Box::new(redact_expr(*right, false)),
        },
        Expr::Prefix { operator, arg } => Expr::Prefix {
            operator,
            arg: Box::new(redact_expr(*arg, false)),
        },
        Expr::Postfix { operator, arg } => Expr::Postfix {
            operator,
            arg: Box::new(redact_expr(*arg, false)),
        },
        Expr::Block(items) => Expr::Block(
            items
                .into_iter()
                .map(|item| redact_expr(item, false))
                .collect(),
        ),
        Expr::Quote { mode, expr } => Expr::Quote {
            mode,
            expr: Box::new(redact_expr(*expr, false)),
        },
        Expr::Annotated { expr, annotations } => Expr::Annotated {
            expr: Box::new(redact_expr(*expr, false)),
            annotations: annotations
                .into_iter()
                .map(|(key, value)| {
                    let sensitive = symbol_is_sensitive(&key);
                    (key, redact_expr(value, sensitive))
                })
                .collect(),
        },
        Expr::Extension { tag, payload } => {
            let sensitive = symbol_is_sensitive(&tag);
            Expr::Extension {
                tag,
                payload: Box::new(redact_expr(*payload, sensitive)),
            }
        }
        other => other,
    }
}

fn sensitive_key(expr: &Expr) -> bool {
    match expr {
        Expr::Symbol(symbol) | Expr::Local(symbol) => symbol_is_sensitive(symbol),
        Expr::String(text) => looks_secret(text),
        _ => false,
    }
}

fn symbol_is_sensitive(symbol: &sim_kernel::Symbol) -> bool {
    looks_secret(&symbol.to_string())
}

fn looks_secret(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    [
        "api-key",
        "apikey",
        "authorization",
        "bearer",
        "password",
        "secret",
        "token",
    ]
    .iter()
    .any(|needle| text.contains(needle))
}

/// Returns the citizen class symbol for [`McpCassetteEntry`].
pub fn mcp_cassette_entry_class_symbol() -> Symbol {
    Symbol::qualified("mcp", "CassetteEntry")
}

/// Returns the citizen class symbol for [`McpAuditEntry`].
pub fn mcp_audit_entry_class_symbol() -> Symbol {
    Symbol::qualified("mcp", "AuditEntry")
}

/// Returns the citizen class symbol for [`McpCassette`].
pub fn mcp_cassette_class_symbol() -> Symbol {
    Symbol::qualified("mcp", "Cassette")
}
