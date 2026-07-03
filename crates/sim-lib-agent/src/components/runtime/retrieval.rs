mod db;
mod query;
mod vector;
mod web;

use super::super::model::{AgentComponent, RetrieverBackend};
use crate::util::expr_to_value;
use sim_kernel::{Cx, Error, Result};
use sim_lib_server::{FrameKind, ServerFrame, eval_request_from_frame};

pub(in crate::components) fn answer_retriever(
    cx: &mut Cx,
    component: &AgentComponent,
    backend: &RetrieverBackend,
    frame: ServerFrame,
) -> Result<ServerFrame> {
    if frame.kind != FrameKind::Request {
        return Err(Error::Eval(format!(
            "{} only answers request frames",
            component.symbol
        )));
    }
    let consistency = frame.envelope.consistency;
    let request = eval_request_from_frame(cx, &frame)?;
    let result = match backend {
        RetrieverBackend::File { root } => db::file_result_expr(cx, root.as_ref(), request.expr)?,
        RetrieverBackend::Vector { store, corpus } => {
            vector::vector_result_expr(cx, store, corpus, request.expr)?
        }
        RetrieverBackend::Web { endpoint } => web::web_result_expr(cx, endpoint, request.expr)?,
        RetrieverBackend::Db { path } => db::db_result_expr(cx, path, request.expr)?,
    };
    let value = expr_to_value(cx, &result)?;
    crate::reply::reply_frame(cx, &frame, value, consistency)
}
