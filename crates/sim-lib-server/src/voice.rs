//! Modeled ASR eval site for watch microphone captures.
//!
//! The site consumes the raw `watch/mic-capture` expression emitted by the watch
//! surface and returns an `asr/transcript` expression. It is an eval fabric, not
//! a watch-local transcription shortcut.

use std::sync::Arc;

use sim_kernel::{
    Consistency, Cx, Error, EvalFabric, EvalMode, EvalReply, EvalRequest, Expr, Result, Symbol,
    eval_remote_capability,
};
use sim_value::{access, build};

use crate::{FabricEvalSite, ServerAddress};

/// Namespace for watch microphone capture expressions.
pub const MIC_CAPTURE_NAMESPACE: &str = "watch";

/// Kind tag for watch microphone capture expressions.
pub const MIC_CAPTURE_KIND: &str = "mic-capture";

/// Namespace for modeled ASR transcript expressions.
pub const ASR_TRANSCRIPT_NAMESPACE: &str = "asr";

/// Kind tag for modeled ASR transcript expressions.
pub const ASR_TRANSCRIPT_KIND: &str = "transcript";

const MODELED_ASR_SITE_KIND: &str = "watch-asr";

/// Deterministic ASR fabric for modeled watch microphone captures.
#[derive(Clone, Debug)]
pub struct ModeledAsrFabric {
    label: String,
}

impl ModeledAsrFabric {
    /// Creates a modeled ASR fabric with a stable transcript label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }
}

impl EvalFabric for ModeledAsrFabric {
    fn realize(&self, cx: &mut Cx, request: EvalRequest) -> Result<EvalReply> {
        if matches!(request.consistency, Consistency::RemoteOnly) {
            return Err(Error::CapabilityDenied {
                capability: eval_remote_capability(),
            });
        }
        if !matches!(request.mode, EvalMode::Eval) {
            return Err(Error::Eval(
                "modeled ASR fabric only supports eval mode".to_owned(),
            ));
        }
        for capability in &request.required_capabilities {
            cx.require(capability)?;
        }

        let capture = MicCaptureView::from_expr(&request.expr)?;
        Ok(EvalReply {
            value: cx.factory().expr(transcript_expr(
                &self.label,
                capture.seq,
                capture.frame_count,
                capture.byte_count,
            ))?,
            diagnostics: cx.take_diagnostics(),
            trace: request
                .trace
                .then(|| {
                    cx.factory()
                        .symbol(Symbol::qualified("asr", "modeled-watch"))
                })
                .transpose()?,
        })
    }
}

/// Builds a modeled ASR eval site over [`ModeledAsrFabric`].
pub fn modeled_asr_site(
    address: ServerAddress,
    codecs: Vec<Symbol>,
    label: impl Into<String>,
) -> FabricEvalSite {
    FabricEvalSite::new(
        MODELED_ASR_SITE_KIND,
        address,
        codecs,
        Arc::new(ModeledAsrFabric::new(label)),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MicCaptureView {
    seq: u64,
    frame_count: usize,
    byte_count: usize,
}

impl MicCaptureView {
    fn from_expr(expr: &Expr) -> Result<Self> {
        ensure_kind(expr, MIC_CAPTURE_KIND, "watch mic capture")?;
        ensure_no_extra(
            expr,
            &["kind", "frames", "seq", "sample-rate-hz", "channels"],
            "watch mic capture",
        )?;
        let frames = match access::required(expr, "frames", "watch mic capture")? {
            Expr::List(items) if !items.is_empty() => items,
            Expr::List(_) => {
                return Err(Error::HostError(
                    "watch mic capture requires at least one frame".to_owned(),
                ));
            }
            _ => {
                return Err(Error::TypeMismatch {
                    expected: "audio frame list",
                    found: "non-list",
                });
            }
        };
        let mut byte_count = 0usize;
        for frame in frames {
            byte_count += frame_byte_count(frame)?;
        }
        Ok(Self {
            seq: uint_field(expr, "seq", "watch mic capture")?,
            frame_count: frames.len(),
            byte_count,
        })
    }
}

fn frame_byte_count(expr: &Expr) -> Result<usize> {
    ensure_kind(expr, "audio-frame", "watch audio frame")?;
    ensure_no_extra(expr, &["kind", "at-ms", "pcm"], "watch audio frame")?;
    match access::required(expr, "pcm", "watch audio frame")? {
        Expr::Bytes(bytes) => Ok(bytes.len()),
        _ => Err(Error::TypeMismatch {
            expected: "raw PCM bytes",
            found: "non-bytes",
        }),
    }
}

fn transcript_expr(label: &str, seq: u64, frame_count: usize, byte_count: usize) -> Expr {
    build::map(vec![
        (
            "kind",
            build::qsym(ASR_TRANSCRIPT_NAMESPACE, ASR_TRANSCRIPT_KIND),
        ),
        (
            "text",
            build::text(format!(
                "{label}: seq {seq}, {frame_count} frame(s), {byte_count} byte(s)"
            )),
        ),
        ("seq", build::uint(seq)),
        ("frames", build::uint(frame_count as u64)),
        ("bytes", build::uint(byte_count as u64)),
    ])
}

fn ensure_kind(expr: &Expr, kind: &str, context: &str) -> Result<()> {
    match access::field_sym(expr, "kind") {
        Some(symbol)
            if symbol.namespace.as_deref() == Some(MIC_CAPTURE_NAMESPACE)
                && symbol.name.as_ref() == kind =>
        {
            Ok(())
        }
        _ => Err(Error::HostError(format!("expected {context}"))),
    }
}

fn ensure_no_extra(expr: &Expr, allowed: &[&str], context: &str) -> Result<()> {
    let Expr::Map(entries) = expr else {
        return Err(Error::HostError(format!("expected {context}")));
    };
    for (key, _) in entries {
        let Expr::Symbol(symbol) = key else {
            return Err(Error::HostError(format!(
                "{context} has a non-symbol field"
            )));
        };
        if symbol.namespace.is_some() || !allowed.contains(&symbol.name.as_ref()) {
            return Err(Error::HostError(format!(
                "{context} has unexpected field {}",
                symbol.as_qualified_str()
            )));
        }
    }
    Ok(())
}

fn uint_field(expr: &Expr, name: &str, context: &str) -> Result<u64> {
    match access::required(expr, name, context)? {
        Expr::Number(number) if number.domain.namespace.is_none() => number
            .canonical
            .parse()
            .map_err(|_| Error::Eval(format!("{context} field {name} is not u64"))),
        _ => Err(Error::Eval(format!("{context} field {name} is not u64"))),
    }
}
