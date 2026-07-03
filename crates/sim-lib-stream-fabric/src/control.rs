use sim_kernel::{CapabilityName, Error, Expr, Result, Symbol};
use sim_lib_server::{
    FrameEnvelope, FrameKind, ServerFrame, stream_chunk_frame_from_expr, stream_frame_to_expr,
};
use sim_lib_stream_core::{
    DataPacket, StreamEnvelope, StreamFaultKind, StreamFaultSpec, StreamMetadata, StreamPacket,
    stream_cancel_capability, stream_open_capability, stream_push_capability,
    stream_read_capability, stream_remote_network_capability, stream_stats_capability,
};

/// Control-plane operation exchanged over remote stream-fabric frames.
///
/// Each variant names one action a peer can take on a placed stream -- opening
/// it, pulling the next chunk, pushing an envelope, ending it, or reporting a
/// fault -- so the location-transparent eval surface can drive a remote stream
/// without exposing transport-specific APIs. A [`StreamControl`] round-trips
/// through the shared `Expr` graph (see [`StreamControl::to_expr`] and the
/// `TryFrom<Expr>` impl) and is carried by [`stream_control_frame_from_control`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamControl {
    /// Open a stream with the given identity and [`StreamMetadata`].
    Open {
        /// Identifier of the stream being opened.
        stream_id: Symbol,
        /// Declared metadata (media, direction, clock domain, buffer policy).
        metadata: StreamMetadata,
    },
    /// Request the next chunk(s), bounded by an optional element limit.
    Next {
        /// Identifier of the stream being pulled.
        stream_id: Symbol,
        /// Maximum number of elements to return, or `None` for unbounded.
        limit: Option<u64>,
    },
    /// Push one stream envelope toward the peer.
    Push {
        /// Identifier of the stream being pushed to.
        stream_id: Symbol,
        /// Boxed envelope carrying the packet, ticks, and transport profile.
        envelope: Box<StreamEnvelope>,
    },
    /// Close a stream cleanly once production is finished.
    Close {
        /// Identifier of the stream being closed.
        stream_id: Symbol,
    },
    /// Cancel a stream early, abandoning any remaining production.
    Cancel {
        /// Identifier of the stream being cancelled.
        stream_id: Symbol,
    },
    /// Request runtime statistics for a stream.
    Stats {
        /// Identifier of the stream being queried.
        stream_id: Symbol,
    },
    /// Request the metadata table for a stream.
    Metadata {
        /// Identifier of the stream being queried.
        stream_id: Symbol,
    },
    /// Report a fault against a stream.
    Fault {
        /// Identifier of the faulting stream.
        stream_id: Symbol,
        /// Fault kind and occurrence count.
        fault: StreamFaultSpec,
    },
}

impl StreamControl {
    /// Returns the operation symbol naming this control variant.
    pub fn operation(&self) -> Symbol {
        match self {
            Self::Open { .. } => stream_control_open_symbol(),
            Self::Next { .. } => stream_control_next_symbol(),
            Self::Push { .. } => stream_control_push_symbol(),
            Self::Close { .. } => stream_control_close_symbol(),
            Self::Cancel { .. } => stream_control_cancel_symbol(),
            Self::Stats { .. } => stream_control_stats_symbol(),
            Self::Metadata { .. } => stream_control_metadata_symbol(),
            Self::Fault { .. } => stream_control_fault_symbol(),
        }
    }

    /// Returns the capability a peer must hold to perform this control action.
    pub fn required_capability(&self) -> CapabilityName {
        match self {
            Self::Open { .. } => stream_open_capability(),
            Self::Next { .. } => stream_read_capability(),
            Self::Push { .. } => stream_push_capability(),
            Self::Close { .. } | Self::Cancel { .. } => stream_cancel_capability(),
            Self::Stats { .. } | Self::Metadata { .. } | Self::Fault { .. } => {
                stream_stats_capability()
            }
        }
    }

    /// Returns the stream identifier targeted by this control action.
    pub fn stream_id(&self) -> &Symbol {
        match self {
            Self::Open { stream_id, .. }
            | Self::Next { stream_id, .. }
            | Self::Push { stream_id, .. }
            | Self::Close { stream_id }
            | Self::Cancel { stream_id }
            | Self::Stats { stream_id }
            | Self::Metadata { stream_id }
            | Self::Fault { stream_id, .. } => stream_id,
        }
    }

    /// Encodes this control action as a stream data packet `Expr`.
    ///
    /// The inverse is the `TryFrom<Expr>` impl, which reparses the packet back
    /// into a [`StreamControl`].
    pub fn to_expr(&self) -> Expr {
        StreamPacket::data(self.operation(), self.payload_expr()).to_expr()
    }

    fn payload_expr(&self) -> Expr {
        let mut entries = vec![
            key_expr("control", Expr::Symbol(stream_control_tag_symbol())),
            key_expr("stream-id", Expr::Symbol(self.stream_id().clone())),
        ];
        match self {
            Self::Open { metadata, .. } => {
                entries.push(key_expr("metadata", metadata.table_expr()));
            }
            Self::Next { limit, .. } => {
                entries.push(key_expr(
                    "limit",
                    limit
                        .map(|limit| Expr::String(limit.to_string()))
                        .unwrap_or(Expr::Nil),
                ));
            }
            Self::Push { envelope, .. } => {
                entries.push(key_expr("envelope", envelope.to_expr()));
            }
            Self::Close { .. }
            | Self::Cancel { .. }
            | Self::Stats { .. }
            | Self::Metadata { .. } => {}
            Self::Fault { fault, .. } => {
                entries.push(key_expr("fault", Expr::Symbol(fault.kind.symbol())));
                entries.push(key_expr("count", Expr::String(fault.count.to_string())));
            }
        }
        Expr::Map(entries)
    }
}

impl TryFrom<Expr> for StreamControl {
    type Error = Error;

    fn try_from(expr: Expr) -> Result<Self> {
        let StreamPacket::Data(DataPacket { kind, payload }) = StreamPacket::try_from(expr)? else {
            return Err(Error::TypeMismatch {
                expected: "stream fabric control data packet",
                found: "stream packet",
            });
        };
        let entries = map_entries(&payload)?;
        let tag = symbol_field(entries, "control")?;
        if *tag != stream_control_tag_symbol() {
            return Err(Error::Eval(format!(
                "unknown stream fabric control tag {}",
                tag.as_qualified_str()
            )));
        }
        let stream_id = symbol_field(entries, "stream-id")?.clone();
        match kind.as_qualified_str().as_str() {
            "stream/fabric/open" => {
                ensure_fields(entries, &["control", "stream-id", "metadata"])?;
                Ok(Self::Open {
                    stream_id,
                    metadata: StreamMetadata::from_table_expr(field(entries, "metadata")?)?,
                })
            }
            "stream/fabric/next" => {
                ensure_fields(entries, &["control", "stream-id", "limit"])?;
                Ok(Self::Next {
                    stream_id,
                    limit: optional_u64(field(entries, "limit")?)?,
                })
            }
            "stream/fabric/push" => {
                ensure_fields(entries, &["control", "stream-id", "envelope"])?;
                Ok(Self::Push {
                    stream_id,
                    envelope: Box::new(StreamEnvelope::try_from(
                        field(entries, "envelope")?.clone(),
                    )?),
                })
            }
            "stream/fabric/close" => {
                ensure_fields(entries, &["control", "stream-id"])?;
                Ok(Self::Close { stream_id })
            }
            "stream/fabric/cancel" => {
                ensure_fields(entries, &["control", "stream-id"])?;
                Ok(Self::Cancel { stream_id })
            }
            "stream/fabric/stats" => {
                ensure_fields(entries, &["control", "stream-id"])?;
                Ok(Self::Stats { stream_id })
            }
            "stream/fabric/metadata" => {
                ensure_fields(entries, &["control", "stream-id"])?;
                Ok(Self::Metadata { stream_id })
            }
            "stream/fabric/fault" => {
                ensure_fields(entries, &["control", "stream-id", "fault", "count"])?;
                Ok(Self::Fault {
                    stream_id,
                    fault: StreamFaultSpec::new(
                        StreamFaultKind::from_symbol(symbol_field(entries, "fault")?)?,
                        parse_u64(field(entries, "count")?)? as usize,
                    ),
                })
            }
            other => Err(Error::Eval(format!(
                "unknown stream fabric control operation {other}"
            ))),
        }
    }
}

/// Encodes a [`StreamControl`] into a server stream-chunk frame.
///
/// Requires the remote-network capability and the action's own
/// [`StreamControl::required_capability`] before encoding the control packet
/// with `codec` into a frame.
pub fn stream_control_frame_from_control(
    cx: &mut sim_kernel::Cx,
    codec: Symbol,
    control: &StreamControl,
    envelope: FrameEnvelope,
) -> Result<ServerFrame> {
    cx.require(&stream_remote_network_capability())?;
    cx.require(&control.required_capability())?;
    stream_chunk_frame_from_expr(cx, codec, &control.to_expr(), envelope)
}

/// Decodes a server stream-chunk frame back into a [`StreamControl`].
///
/// Returns an error when the frame is not a stream-chunk frame or does not
/// decode to a control payload.
pub fn stream_control_from_frame(
    cx: &mut sim_kernel::Cx,
    frame: &ServerFrame,
) -> Result<StreamControl> {
    if frame.kind != FrameKind::StreamChunk {
        return Err(Error::Eval(format!(
            "stream fabric control expected stream chunk frame, got {}",
            frame.kind.as_symbol()
        )));
    }
    let expr = stream_frame_to_expr(cx, frame)?.ok_or_else(|| {
        Error::Eval("stream fabric control frame did not decode to a payload".to_owned())
    })?;
    StreamControl::try_from(expr)
}

/// Returns every stream-fabric control operation symbol, one per variant.
pub fn stream_control_operation_symbols() -> [Symbol; 8] {
    [
        stream_control_open_symbol(),
        stream_control_next_symbol(),
        stream_control_push_symbol(),
        stream_control_close_symbol(),
        stream_control_cancel_symbol(),
        stream_control_stats_symbol(),
        stream_control_metadata_symbol(),
        stream_control_fault_symbol(),
    ]
}

/// Returns the capability required to perform `control`.
///
/// Free-function form of [`StreamControl::required_capability`].
pub fn stream_control_required_capability(control: &StreamControl) -> CapabilityName {
    control.required_capability()
}

/// Returns the operation symbol for the open control action.
pub fn stream_control_open_symbol() -> Symbol {
    Symbol::qualified("stream/fabric", "open")
}

/// Returns the operation symbol for the next control action.
pub fn stream_control_next_symbol() -> Symbol {
    Symbol::qualified("stream/fabric", "next")
}

/// Returns the operation symbol for the push control action.
pub fn stream_control_push_symbol() -> Symbol {
    Symbol::qualified("stream/fabric", "push")
}

/// Returns the operation symbol for the close control action.
pub fn stream_control_close_symbol() -> Symbol {
    Symbol::qualified("stream/fabric", "close")
}

/// Returns the operation symbol for the cancel control action.
pub fn stream_control_cancel_symbol() -> Symbol {
    Symbol::qualified("stream/fabric", "cancel")
}

/// Returns the operation symbol for the stats control action.
pub fn stream_control_stats_symbol() -> Symbol {
    Symbol::qualified("stream/fabric", "stats")
}

/// Returns the operation symbol for the metadata control action.
pub fn stream_control_metadata_symbol() -> Symbol {
    Symbol::qualified("stream/fabric", "metadata")
}

/// Returns the operation symbol for the fault control action.
pub fn stream_control_fault_symbol() -> Symbol {
    Symbol::qualified("stream/fabric", "fault")
}

fn stream_control_tag_symbol() -> Symbol {
    Symbol::qualified("stream/fabric-control", "v1")
}

fn key_expr(name: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(name)), value)
}

fn map_entries(expr: &Expr) -> Result<&[(Expr, Expr)]> {
    match expr {
        Expr::Map(entries) => Ok(entries),
        other => Err(Error::TypeMismatch {
            expected: "stream fabric control map",
            found: expr_kind(other),
        }),
    }
}

fn optional_u64(expr: &Expr) -> Result<Option<u64>> {
    match expr {
        Expr::Nil => Ok(None),
        Expr::String(value) => value
            .parse::<u64>()
            .map(Some)
            .map_err(|err| Error::Eval(format!("invalid stream fabric control limit: {err}"))),
        other => Err(Error::TypeMismatch {
            expected: "optional u64 string",
            found: expr_kind(other),
        }),
    }
}

fn parse_u64(expr: &Expr) -> Result<u64> {
    match expr {
        Expr::String(value) => value
            .parse::<u64>()
            .map_err(|err| Error::Eval(format!("invalid stream fabric control count: {err}"))),
        other => Err(Error::TypeMismatch {
            expected: "u64 string",
            found: expr_kind(other),
        }),
    }
}

fn symbol_field<'a>(entries: &'a [(Expr, Expr)], name: &str) -> Result<&'a Symbol> {
    match field(entries, name)? {
        Expr::Symbol(symbol) => Ok(symbol),
        other => Err(Error::TypeMismatch {
            expected: "symbol field",
            found: expr_kind(other),
        }),
    }
}

fn field<'a>(entries: &'a [(Expr, Expr)], name: &str) -> Result<&'a Expr> {
    entries
        .iter()
        .find_map(|(key, value)| match key {
            Expr::Symbol(symbol) if symbol.namespace.is_none() && symbol.name.as_ref() == name => {
                Some(value)
            }
            _ => None,
        })
        .ok_or_else(|| Error::Eval(format!("stream fabric control missing {name} field")))
}

fn ensure_fields(entries: &[(Expr, Expr)], allowed: &[&str]) -> Result<()> {
    for (key, _) in entries {
        let Expr::Symbol(symbol) = key else {
            return Err(Error::TypeMismatch {
                expected: "symbol stream fabric control field",
                found: expr_kind(key),
            });
        };
        if symbol.namespace.is_none() && allowed.contains(&symbol.name.as_ref()) {
            continue;
        }
        return Err(Error::Eval(format!(
            "unknown stream fabric control field {}",
            symbol.as_qualified_str()
        )));
    }
    Ok(())
}

fn expr_kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::Nil => "nil",
        Expr::Bool(_) => "bool",
        Expr::Number(_) => "number",
        Expr::Symbol(_) => "symbol",
        Expr::Local(_) => "local",
        Expr::String(_) => "string",
        Expr::Bytes(_) => "bytes",
        Expr::List(_) => "list",
        Expr::Vector(_) => "vector",
        Expr::Map(_) => "map",
        Expr::Set(_) => "set",
        Expr::Call { .. } => "call",
        Expr::Infix { .. } => "infix",
        Expr::Prefix { .. } => "prefix",
        Expr::Postfix { .. } => "postfix",
        Expr::Block(_) => "block",
        Expr::Quote { .. } => "quote",
        Expr::Annotated { .. } => "annotated",
        Expr::Extension { .. } => "extension",
    }
}
