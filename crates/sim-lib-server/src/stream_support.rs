use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use sim_kernel::{Args, ClassRef, Cx, Error, Expr, Object, Result, Stream, Symbol, Value};

use crate::{FrameEnvelope, FrameKind, ServerFrame, StreamSink, eval_reply_from_frame};

pub(crate) fn stream_handle_from_value(value: &Value) -> Option<&StreamHandle> {
    value.object().downcast_ref::<StreamHandle>()
}

pub(crate) fn evaluated_stream_handle(cx: &mut Cx, expr: &Expr) -> Result<StreamHandle> {
    let value = cx.eval_expr(expr.clone())?;
    stream_handle_from_value(&value)
        .cloned()
        .ok_or(Error::TypeMismatch {
            expected: "stream handle",
            found: "non-stream",
        })
}

pub(crate) fn stream_handle_arg<'a>(
    args: &'a Args,
    index: usize,
    message: &'static str,
) -> Result<&'a StreamHandle> {
    let Some(value) = args.values().get(index) else {
        return Err(Error::Eval(message.to_owned()));
    };
    value
        .object()
        .downcast_ref::<StreamHandle>()
        .ok_or(Error::TypeMismatch {
            expected: "stream handle",
            found: "non-stream",
        })
}

#[derive(Clone, Default)]
/// Buffering, cancellable handle over a server stream's chunk queue.
///
/// Cloning shares the same underlying buffer; usable as a runtime stream
/// object.
pub struct StreamHandle {
    state: Arc<Mutex<StreamState>>,
}

#[derive(Default)]
struct StreamState {
    chunks: VecDeque<Value>,
    done: bool,
    cancelled: bool,
    error: Option<String>,
}

impl StreamHandle {
    pub(crate) fn push(&self, value: Value) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("stream handle"))?;
        if !state.cancelled && !state.done {
            state.chunks.push_back(value);
        }
        Ok(())
    }

    pub(crate) fn finish(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("stream handle"))?;
        state.done = true;
        Ok(())
    }

    pub(crate) fn finish_with_error(&self, message: String) {
        if let Ok(mut state) = self.state.lock() {
            state.error = Some(message);
            state.done = true;
        }
    }

    fn next_item(&self, _cx: &mut Cx) -> Result<Option<Value>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::PoisonedLock("stream handle"))?;
        if let Some(error) = state.error.take() {
            return Err(Error::Eval(error));
        }
        Ok(state.chunks.pop_front())
    }

    pub(crate) fn next(&self, cx: &mut Cx) -> Result<Value> {
        Ok(self.next_item(cx)?.unwrap_or(cx.factory().nil()?))
    }

    pub(crate) fn cancel(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.cancelled = true;
            state.done = true;
            state.chunks.clear();
        }
    }

    pub(crate) fn is_done(&self) -> bool {
        self.state.lock().map(|state| state.done).unwrap_or(true)
    }

    pub(crate) fn buffered_len(&self) -> usize {
        self.state
            .lock()
            .map(|state| state.chunks.len())
            .unwrap_or(0)
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.cancelled)
            .unwrap_or(true)
    }

    /// Returns a snapshot copy of the currently buffered chunk values.
    pub fn buffered_values(&self) -> Result<Vec<Value>> {
        self.state
            .lock()
            .map(|state| state.chunks.iter().cloned().collect())
            .map_err(|_| Error::PoisonedLock("stream handle"))
    }
}

impl Object for StreamHandle {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<server-stream>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for StreamHandle {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            sim_kernel::ClassId(0),
            Symbol::qualified("server", "StreamHandle"),
        )
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table(cx)?.object().as_expr(cx)
    }
    fn as_stream(&self) -> Option<&dyn Stream> {
        Some(self)
    }
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        cx.factory().table(vec![
            (
                Symbol::new("kind"),
                cx.factory().symbol(Symbol::new("stream-handle"))?,
            ),
            (
                Symbol::new("buffered"),
                cx.factory().string(self.buffered_len().to_string())?,
            ),
            (Symbol::new("done"), cx.factory().bool(self.is_done())?),
            (
                Symbol::new("cancelled"),
                cx.factory().bool(self.is_cancelled())?,
            ),
        ])
    }
}

impl Stream for StreamHandle {
    fn next(&self, cx: &mut Cx) -> Result<Option<Value>> {
        self.next_item(cx)
    }

    fn close(&self, _cx: &mut Cx) -> Result<()> {
        self.cancel();
        Ok(())
    }
}

/// Converts a stream frame into the value it carries, if any.
///
/// Response and stream-chunk frames yield a value; stream start/end frames
/// yield `None`; other frame kinds are an error.
pub fn stream_frame_to_value(cx: &mut Cx, frame: &ServerFrame) -> Result<Option<Value>> {
    match &frame.kind {
        FrameKind::Response => Ok(Some(eval_reply_from_frame(cx, frame)?.value)),
        FrameKind::StreamChunk => {
            let expr = stream_frame_to_expr(cx, frame)?.ok_or_else(|| {
                Error::Eval("stream chunk frame did not decode to a payload".to_owned())
            })?;
            Ok(Some(cx.eval_expr(expr)?))
        }
        FrameKind::StreamStart | FrameKind::StreamEnd => Ok(None),
        _ => Err(unsupported_stream_frame_error(&frame.kind)),
    }
}

/// Converts a stream frame into the expression it carries, if any.
///
/// Response and stream-chunk frames yield an expression; stream start/end
/// frames yield `None`; other frame kinds are an error.
pub fn stream_frame_to_expr(cx: &mut Cx, frame: &ServerFrame) -> Result<Option<Expr>> {
    match &frame.kind {
        FrameKind::Response => {
            let value = eval_reply_from_frame(cx, frame)?.value;
            Ok(Some(value.object().as_expr(cx)?))
        }
        FrameKind::StreamChunk => Ok(Some(
            frame.decode_expr(cx, sim_kernel::ReadPolicy::default())?,
        )),
        FrameKind::StreamStart | FrameKind::StreamEnd => Ok(None),
        _ => Err(unsupported_stream_frame_error(&frame.kind)),
    }
}

/// Builds a stream boundary or chunk frame from an expression payload.
///
/// New stream-producing code should use this helper for `StreamStart` and
/// `StreamChunk` frames so envelope handling stays consistent across server,
/// agent, and stream-fabric adapters.
pub fn stream_frame_from_expr(
    cx: &mut Cx,
    codec: Symbol,
    kind: FrameKind,
    expr: &Expr,
    envelope: FrameEnvelope,
) -> Result<ServerFrame> {
    match kind {
        FrameKind::StreamStart | FrameKind::StreamChunk => {
            let mut frame = ServerFrame::from_expr(
                cx,
                codec,
                kind,
                expr,
                envelope.consistency,
                envelope.required_capabilities.clone(),
                envelope.trace,
            )?;
            frame.envelope = envelope;
            Ok(frame)
        }
        _ => Err(Error::Eval(format!(
            "stream expression frame helper cannot build frame kind {}",
            kind.as_symbol()
        ))),
    }
}

/// Builds the standard `StreamChunk` frame for one expression payload.
pub fn stream_chunk_frame_from_expr(
    cx: &mut Cx,
    codec: Symbol,
    expr: &Expr,
    envelope: FrameEnvelope,
) -> Result<ServerFrame> {
    stream_frame_from_expr(cx, codec, FrameKind::StreamChunk, expr, envelope)
}

/// Builds the standard `StreamEnd` frame for a stream envelope.
pub fn stream_end_frame(codec: Symbol, envelope: FrameEnvelope) -> ServerFrame {
    ServerFrame::new(codec, FrameKind::StreamEnd, envelope, Vec::new())
}

fn unsupported_stream_frame_error(kind: &FrameKind) -> Error {
    Error::Eval(format!(
        "stream sink cannot buffer frame kind {}",
        kind.as_symbol()
    ))
}

/// [`StreamSink`] that buffers received chunks into a [`StreamHandle`].
pub struct BufferedStreamSink {
    handle: Arc<StreamHandle>,
}

impl BufferedStreamSink {
    /// Creates a sink that pushes chunks into `handle`.
    pub fn new(handle: Arc<StreamHandle>) -> Self {
        Self { handle }
    }
}

impl StreamSink for BufferedStreamSink {
    fn chunk(&mut self, cx: &mut Cx, frame: ServerFrame) -> Result<()> {
        let closes_stream = matches!(frame.kind, FrameKind::StreamEnd);
        if let Some(value) = stream_frame_to_value(cx, &frame)? {
            self.handle.push(value)?;
        }
        if closes_stream {
            self.handle.finish()?;
        }
        Ok(())
    }

    fn end(&mut self, _cx: &mut Cx) -> Result<()> {
        self.handle.finish()
    }
}

#[cfg(test)]
mod tests;
