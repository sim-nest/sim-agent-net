use std::path::PathBuf;

use sim_kernel::{Cx, Error, Expr, Result, Symbol, Value};

/// Location of a SIM server endpoint, parsed from an address expression.
///
/// Identifies where eval/agent traffic is served -- in-process, over a
/// transport, through an integration, or as a pipeline of stages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerAddress {
    /// The current process; no transport is involved.
    Local,
    /// A peer thread within the same process.
    InProcess {
        /// Identifier of the target thread.
        thread: u64,
    },
    /// A coroutine within the local scheduler.
    Coroutine {
        /// Identifier of the target coroutine.
        id: u64,
    },
    /// A TCP endpoint.
    Tcp {
        /// Host name or address to connect to.
        host: String,
        /// TCP port number.
        port: u16,
    },
    /// A Unix domain socket.
    Unix {
        /// Filesystem path of the socket.
        path: PathBuf,
    },
    /// A wasm guest region.
    Wasm {
        /// Name of the target wasm region.
        region: String,
    },
    /// An HTTP endpoint.
    Http {
        /// URL to reach the endpoint.
        url: String,
    },
    /// A WebSocket endpoint.
    Ws {
        /// URL to reach the endpoint.
        url: String,
    },
    /// A server-sent events endpoint.
    Sse {
        /// URL to reach the endpoint.
        url: String,
    },
    /// An SMTP mail endpoint.
    Smtp {
        /// Mail address to send to.
        address: String,
    },
    /// An IMAP mailbox endpoint.
    Imap {
        /// Mail address of the account.
        address: String,
        /// Mailbox name to read.
        mailbox: String,
    },
    /// A Telegram bot chat endpoint.
    Telegram {
        /// Identifier of the target chat.
        chat_id: String,
        /// Bot account used to send.
        bot: String,
    },
    /// A Matrix room endpoint.
    Matrix {
        /// Identifier of the target room.
        room_id: String,
    },
    /// Standard input as a source.
    Stdin,
    /// A file followed for appended lines.
    FileTail {
        /// Filesystem path of the file to tail.
        path: PathBuf,
    },
    /// A cron schedule that triggers serving.
    Cron {
        /// Cron schedule specification.
        spec: String,
    },
    /// An inbound webhook route.
    Webhook {
        /// Route the webhook is mounted at.
        route: String,
    },
    /// A named agent endpoint.
    Agent {
        /// Identifier or name of the target agent.
        agent: String,
    },
    /// A sequence of addresses chained as stages.
    Pipeline {
        /// Ordered stages making up the pipeline.
        steps: Vec<ServerAddress>,
    },
    /// A wildcard address matching any endpoint.
    Any,
}

impl ServerAddress {
    /// Returns `true` when the address denotes a non-local endpoint.
    ///
    /// `Local` and `Any` are local; a `Pipeline` is remote-like if any of its
    /// stages is.
    pub fn is_remote_like(&self) -> bool {
        match self {
            Self::Local | Self::Any => false,
            Self::Pipeline { steps } => steps.iter().any(Self::is_remote_like),
            _ => true,
        }
    }

    /// Parses a [`ServerAddress`] from an address expression.
    ///
    /// A bare symbol selects `local`, `stdin`, or `any`; a list or vector is
    /// read as a kind symbol followed by key/value option pairs.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        match expr {
            Expr::Symbol(symbol) => match symbol.name.as_ref() {
                "local" => Ok(Self::Local),
                "stdin" => Ok(Self::Stdin),
                "any" => Ok(Self::Any),
                _ => Err(Error::Eval(format!(
                    "unsupported server address symbol {}",
                    symbol
                ))),
            },
            Expr::List(items) | Expr::Vector(items) => Self::from_items(items),
            _ => Err(Error::TypeMismatch {
                expected: "server address expression",
                found: "non-address",
            }),
        }
    }

    /// Returns the symbol naming this address kind (e.g. `tcp`, `pipeline`).
    pub fn kind_symbol(&self) -> Symbol {
        Symbol::new(match self {
            Self::Local => "local",
            Self::InProcess { .. } => "in-process",
            Self::Coroutine { .. } => "coroutine",
            Self::Tcp { .. } => "tcp",
            Self::Unix { .. } => "unix",
            Self::Wasm { .. } => "wasm",
            Self::Http { .. } => "http",
            Self::Ws { .. } => "ws",
            Self::Sse { .. } => "sse",
            Self::Smtp { .. } => "smtp",
            Self::Imap { .. } => "imap",
            Self::Telegram { .. } => "telegram",
            Self::Matrix { .. } => "matrix",
            Self::Stdin => "stdin",
            Self::FileTail { .. } => "file-tail",
            Self::Cron { .. } => "cron",
            Self::Webhook { .. } => "webhook",
            Self::Agent { .. } => "agent",
            Self::Pipeline { .. } => "pipeline",
            Self::Any => "any",
        })
    }

    /// Builds a runtime table value describing the address.
    ///
    /// The table carries a `kind` entry plus one entry per field of the
    /// variant; a `Pipeline` nests its stages as a list of such tables.
    pub fn as_value(&self, cx: &mut Cx) -> Result<Value> {
        let mut entries = vec![(
            Symbol::new("kind"),
            cx.factory().symbol(self.kind_symbol())?,
        )];
        match self {
            Self::Local | Self::Stdin | Self::Any => {}
            Self::InProcess { thread } => {
                entries.push((
                    Symbol::new("thread"),
                    cx.factory().string(thread.to_string())?,
                ));
            }
            Self::Coroutine { id } => {
                entries.push((Symbol::new("id"), cx.factory().string(id.to_string())?));
            }
            Self::Tcp { host, port } => {
                entries.push((Symbol::new("host"), cx.factory().string(host.clone())?));
                entries.push((Symbol::new("port"), cx.factory().string(port.to_string())?));
            }
            Self::Unix { path } | Self::FileTail { path } => {
                entries.push((
                    Symbol::new("path"),
                    cx.factory().string(path.display().to_string())?,
                ));
            }
            Self::Wasm { region } => {
                entries.push((Symbol::new("region"), cx.factory().string(region.clone())?));
            }
            Self::Http { url } | Self::Ws { url } | Self::Sse { url } => {
                entries.push((Symbol::new("url"), cx.factory().string(url.clone())?));
            }
            Self::Smtp { address } => {
                entries.push((
                    Symbol::new("address"),
                    cx.factory().string(address.clone())?,
                ));
            }
            Self::Imap { address, mailbox } => {
                entries.push((
                    Symbol::new("address"),
                    cx.factory().string(address.clone())?,
                ));
                entries.push((
                    Symbol::new("mailbox"),
                    cx.factory().string(mailbox.clone())?,
                ));
            }
            Self::Telegram { chat_id, bot } => {
                entries.push((
                    Symbol::new("chat-id"),
                    cx.factory().string(chat_id.clone())?,
                ));
                entries.push((Symbol::new("bot"), cx.factory().string(bot.clone())?));
            }
            Self::Matrix { room_id } => {
                entries.push((
                    Symbol::new("room-id"),
                    cx.factory().string(room_id.clone())?,
                ));
            }
            Self::Cron { spec } => {
                entries.push((Symbol::new("spec"), cx.factory().string(spec.clone())?));
            }
            Self::Webhook { route } => {
                entries.push((Symbol::new("route"), cx.factory().string(route.clone())?));
            }
            Self::Agent { agent } => {
                entries.push((Symbol::new("agent"), cx.factory().string(agent.clone())?));
            }
            Self::Pipeline { steps } => {
                let values = steps
                    .iter()
                    .map(|step| step.as_value(cx))
                    .collect::<Result<Vec<_>>>()?;
                entries.push((Symbol::new("steps"), cx.factory().list(values)?));
            }
        }
        cx.factory().table(entries)
    }

    /// Returns `true` when a transport is implemented for this address kind.
    pub fn transport_available(&self) -> bool {
        matches!(
            self,
            Self::Local
                | Self::Any
                | Self::Pipeline { .. }
                | Self::InProcess { .. }
                | Self::Coroutine { .. }
                | Self::Tcp { .. }
                | Self::Unix { .. }
                | Self::Wasm { .. }
                | Self::Http { .. }
                | Self::Ws { .. }
                | Self::Sse { .. }
                | Self::Agent { .. }
        )
    }

    /// Succeeds when a transport exists for this address, else returns an error.
    pub fn ensure_transport_available(&self) -> Result<()> {
        if self.transport_available() {
            Ok(())
        } else {
            Err(Error::Eval(format!(
                "no transport for address kind {}",
                self.kind_symbol()
            )))
        }
    }

    fn from_items(items: &[Expr]) -> Result<Self> {
        let Some(Expr::Symbol(kind)) = items.first() else {
            return Err(Error::TypeMismatch {
                expected: "address list starting with a symbol",
                found: "non-symbol",
            });
        };
        match kind.name.as_ref() {
            "in-process" => {
                let thread = find_u64(items, "thread")?.unwrap_or(0);
                Ok(Self::InProcess { thread })
            }
            "coroutine" => {
                let id = find_u64(items, "id")?.unwrap_or(0);
                Ok(Self::Coroutine { id })
            }
            "tcp" => Ok(Self::Tcp {
                host: find_string(items, "host")?.unwrap_or_else(|| "127.0.0.1".to_owned()),
                port: find_u16(items, "port")?
                    .ok_or_else(|| Error::Eval("tcp address requires :port".to_owned()))?,
            }),
            "unix" => Ok(Self::Unix {
                path: PathBuf::from(
                    find_string(items, "path")?
                        .ok_or_else(|| Error::Eval("unix address requires :path".to_owned()))?,
                ),
            }),
            "wasm" => Ok(Self::Wasm {
                region: find_string(items, "region")?
                    .ok_or_else(|| Error::Eval("wasm address requires :region".to_owned()))?,
            }),
            "http" => Ok(Self::Http {
                url: find_string(items, "url")?
                    .ok_or_else(|| Error::Eval("http address requires :url".to_owned()))?,
            }),
            "ws" => Ok(Self::Ws {
                url: find_string(items, "url")?
                    .ok_or_else(|| Error::Eval("ws address requires :url".to_owned()))?,
            }),
            "sse" => Ok(Self::Sse {
                url: find_string(items, "url")?
                    .ok_or_else(|| Error::Eval("sse address requires :url".to_owned()))?,
            }),
            "smtp" => Ok(Self::Smtp {
                address: find_string(items, "address")?
                    .ok_or_else(|| Error::Eval("smtp address requires :address".to_owned()))?,
            }),
            "imap" => Ok(Self::Imap {
                address: find_string(items, "address")?
                    .ok_or_else(|| Error::Eval("imap address requires :address".to_owned()))?,
                mailbox: find_string(items, "mailbox")?
                    .ok_or_else(|| Error::Eval("imap address requires :mailbox".to_owned()))?,
            }),
            "telegram" => Ok(Self::Telegram {
                chat_id: find_string(items, "chat-id")?
                    .or_else(|| find_string(items, "chat").ok().flatten())
                    .ok_or_else(|| Error::Eval("telegram address requires :chat-id".to_owned()))?,
                bot: find_string(items, "bot")?
                    .ok_or_else(|| Error::Eval("telegram address requires :bot".to_owned()))?,
            }),
            "matrix" => Ok(Self::Matrix {
                room_id: find_string(items, "room-id")?
                    .or_else(|| find_string(items, "room").ok().flatten())
                    .ok_or_else(|| Error::Eval("matrix address requires :room-id".to_owned()))?,
            }),
            "file-tail" => {
                Ok(Self::FileTail {
                    path: PathBuf::from(find_string(items, "path")?.ok_or_else(|| {
                        Error::Eval("file-tail address requires :path".to_owned())
                    })?),
                })
            }
            "cron" => Ok(Self::Cron {
                spec: find_string(items, "spec")?
                    .ok_or_else(|| Error::Eval("cron address requires :spec".to_owned()))?,
            }),
            "webhook" => Ok(Self::Webhook {
                route: find_string(items, "route")?
                    .ok_or_else(|| Error::Eval("webhook address requires :route".to_owned()))?,
            }),
            "agent" => {
                let agent = items
                    .get(1)
                    .ok_or_else(|| Error::Eval("agent address requires a target".to_owned()))?;
                Ok(Self::Agent {
                    agent: stringy(agent)?,
                })
            }
            "pipeline" => Ok(Self::Pipeline {
                steps: items[1..]
                    .iter()
                    .map(Self::from_expr)
                    .collect::<Result<Vec<_>>>()?,
            }),
            other => Err(Error::Eval(format!(
                "unsupported server address kind {other}"
            ))),
        }
    }
}

fn find_expr<'a>(items: &'a [Expr], key: &str) -> Result<Option<&'a Expr>> {
    if items.len() <= 1 {
        return Ok(None);
    }
    if !(items.len() - 1).is_multiple_of(2) {
        return Err(Error::Eval(
            "address options must be key/value pairs".to_owned(),
        ));
    }
    for pair in items[1..].chunks(2) {
        let Expr::Symbol(symbol) = &pair[0] else {
            return Err(Error::TypeMismatch {
                expected: "keyword symbol",
                found: "non-symbol",
            });
        };
        let name = symbol
            .name
            .strip_prefix(':')
            .unwrap_or(symbol.name.as_ref());
        if name == key {
            return Ok(Some(&pair[1]));
        }
    }
    Ok(None)
}

fn find_string(items: &[Expr], key: &str) -> Result<Option<String>> {
    find_expr(items, key)?.map(stringy).transpose()
}

fn find_u64(items: &[Expr], key: &str) -> Result<Option<u64>> {
    find_expr(items, key)?.map(integer_u64).transpose()
}

fn find_u16(items: &[Expr], key: &str) -> Result<Option<u16>> {
    find_u64(items, key)?
        .map(|value| {
            u16::try_from(value)
                .map_err(|_| Error::Eval(format!("address field :{key} value {value} exceeds u16")))
        })
        .transpose()
}

fn stringy(expr: &Expr) -> Result<String> {
    match expr {
        Expr::String(text) => Ok(text.clone()),
        Expr::Symbol(symbol) => Ok(symbol.to_string()),
        _ => Err(Error::TypeMismatch {
            expected: "string or symbol",
            found: "non-string",
        }),
    }
}

fn integer_u64(expr: &Expr) -> Result<u64> {
    match expr {
        Expr::Number(number) => number.canonical.parse::<u64>().map_err(|_| {
            Error::Eval(format!(
                "{} is not a valid unsigned integer",
                number.canonical
            ))
        }),
        Expr::String(text) => text
            .parse::<u64>()
            .map_err(|_| Error::Eval(format!("{text} is not a valid unsigned integer"))),
        _ => Err(Error::TypeMismatch {
            expected: "integer number or string",
            found: "non-integer",
        }),
    }
}
