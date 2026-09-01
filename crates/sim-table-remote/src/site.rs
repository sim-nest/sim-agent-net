use std::sync::Arc;

use sim_kernel::{
    Consistency, Cx, Error, EvalReply, Expr, Result, Symbol, TableExpected, TableObserved,
    TableReplacement, Value,
};
use sim_lib_server::{
    EvalSite, FrameKind, ServerAddress, ServerFrame, eval_request_from_frame,
    server_frame_from_reply,
};
use sim_table_core::{CompareExpected, CompareReplacement, TableOp, TableOpError, decode_table_op};

/// Wraps `inner` so table operations dispatch against the table `root`.
pub fn wrap_remote_table_site(inner: Arc<dyn EvalSite>, root: Value) -> Arc<dyn EvalSite> {
    Arc::new(RemoteTableSite { inner, root })
}

/// [`EvalSite`] adapter that serves table operations from a local `root` table.
#[derive(Clone)]
pub struct RemoteTableSite {
    inner: Arc<dyn EvalSite>,
    root: Value,
}

impl RemoteTableSite {
    fn current_dir(&self, cx: &mut Cx, path: &[Symbol]) -> Result<Value> {
        let mut current = self.root.clone();
        for segment in path {
            let dir = current.object().as_dir().ok_or(Error::TypeMismatch {
                expected: "directory table",
                found: "non-directory",
            })?;
            current = dir.opendir(cx, segment.clone())?.ok_or_else(|| {
                Error::Eval(format!("table/remote: missing path segment {segment}"))
            })?;
        }
        Ok(current)
    }

    fn parse_call(expr: Expr) -> Result<(Symbol, Vec<Expr>)> {
        match expr {
            Expr::Call { operator, args } => match *operator {
                Expr::Symbol(symbol) => Ok((symbol, args)),
                _ => Err(Error::TypeMismatch {
                    expected: "symbol operator",
                    found: "non-symbol",
                }),
            },
            _ => Err(Error::TypeMismatch {
                expected: "call",
                found: "non-call",
            }),
        }
    }

    fn parse_path(expr: &Expr) -> Result<Vec<Symbol>> {
        let Expr::List(items) = expr else {
            return Err(Error::TypeMismatch {
                expected: "path list",
                found: "non-list",
            });
        };
        items
            .iter()
            .map(|item| match item {
                Expr::Symbol(symbol) => Ok(symbol.clone()),
                _ => Err(Error::TypeMismatch {
                    expected: "path symbol",
                    found: "non-symbol",
                }),
            })
            .collect()
    }

    /// The arity-error message for a known wire op name, preserving the exact
    /// strings the per-op dispatch used before sharing the `TableOp` codec.
    fn arity_message(name: &str) -> String {
        match name {
            "get" => "table/get expects path and key".to_owned(),
            "set" => "table/set expects path, key, and value".to_owned(),
            "cas" => "table/cas expects path, key, expected, and replacement".to_owned(),
            "has" => "table/has expects path and key".to_owned(),
            "del" => "table/del expects path and key".to_owned(),
            "keys" => "table/keys expects only a path".to_owned(),
            "entries" => "table/entries expects only a path".to_owned(),
            "len" => "table/len expects only a path".to_owned(),
            "clear" => "table/clear expects only a path".to_owned(),
            "mkdir" => "table/mkdir expects path and name".to_owned(),
            "opendir" => "table/opendir expects path and name".to_owned(),
            "rmdir" => "table/rmdir expects path and name".to_owned(),
            "dir?" => "table/dir? expects path and name".to_owned(),
            other => format!("table/{other}: malformed request"),
        }
    }

    fn child_token(path: &[Symbol], name: &Symbol) -> String {
        let mut segments = path.iter().map(Symbol::to_string).collect::<Vec<_>>();
        segments.push(name.to_string());
        format!("/{}", segments.join("/"))
    }

    fn reply_value(
        &self,
        cx: &mut Cx,
        frame: &ServerFrame,
        value: Value,
        consistency: Consistency,
    ) -> Result<ServerFrame> {
        let reply = EvalReply {
            value,
            diagnostics: cx.take_diagnostics(),
            trace: None,
        };
        server_frame_from_reply(cx, &self.reply_codec(frame), reply, consistency)
    }

    fn reply_codec(&self, frame: &ServerFrame) -> Symbol {
        if let Some(hint) = &frame.envelope.reply_codec_hint
            && self.inner.codecs().iter().any(|codec| codec == hint)
        {
            return hint.clone();
        }
        frame.codec.clone()
    }

    fn answer_table_request(&self, cx: &mut Cx, frame: ServerFrame) -> Result<Option<ServerFrame>> {
        if frame.kind != FrameKind::Request {
            return Ok(None);
        }
        let consistency = frame.envelope.consistency;
        let request = eval_request_from_frame(cx, &frame)?;
        let (operator, args) = Self::parse_call(request.expr)?;
        if operator.namespace.as_deref() != Some("table") {
            return Ok(None);
        }
        let [path_expr, rest @ ..] = args.as_slice() else {
            return Err(Error::Eval(
                "table/remote: missing path argument".to_owned(),
            ));
        };
        let path = Self::parse_path(path_expr)?;

        // Parse the op against the one shared `table/<op>` vocabulary. The path
        // is transport context, so we decode a Call carrying just `rest`. An
        // unknown table op falls through to the inner site exactly as before; a
        // malformed-but-known op reproduces the original arity/arg error.
        let op = match decode_table_op(&Expr::Call {
            operator: Box::new(Expr::Symbol(operator.clone())),
            args: rest.to_vec(),
        }) {
            Ok(op) => op,
            Err(TableOpError::UnknownOp(_) | TableOpError::NotATableCall) => return Ok(None),
            Err(TableOpError::BadArity(name)) => {
                return Err(Error::Eval(Self::arity_message(&name)));
            }
            Err(TableOpError::BadArg(_)) => {
                return Err(Error::TypeMismatch {
                    expected: "symbol",
                    found: "non-symbol",
                });
            }
        };

        let current = self.current_dir(cx, &path)?;
        let table = current
            .object()
            .as_table_impl()
            .ok_or(Error::TypeMismatch {
                expected: "table",
                found: "non-table",
            })?;
        let value = match op {
            TableOp::Get(key) => table.get(cx, key),
            TableOp::Set(key, value) => {
                table.set(cx, key, cx.factory().expr(value)?)?;
                cx.factory().nil()
            }
            TableOp::CompareExchange(key, expected, replacement) => {
                let expected = match expected {
                    CompareExpected::Absent => TableExpected::Absent,
                    CompareExpected::Value(expr) => TableExpected::Value(expr),
                };
                let replacement = match replacement {
                    CompareReplacement::Delete => TableReplacement::Delete,
                    CompareReplacement::Value(expr) => {
                        TableReplacement::Value(cx.factory().expr(expr)?)
                    }
                };
                let outcome = table.compare_exchange(cx, key, expected, replacement)?;
                let observed = match outcome.observed {
                    TableObserved::Absent => Expr::Call {
                        operator: Box::new(Expr::Symbol(Symbol::qualified("table", "absent"))),
                        args: vec![],
                    },
                    TableObserved::Value(expr) => Expr::Call {
                        operator: Box::new(Expr::Symbol(Symbol::qualified("table", "value"))),
                        args: vec![expr],
                    },
                };
                cx.factory().expr(Expr::Call {
                    operator: Box::new(Expr::Symbol(Symbol::qualified("table", "cas-result"))),
                    args: vec![Expr::Bool(outcome.exchanged), observed],
                })
            }
            TableOp::Has(key) => {
                let present = table.has(cx, key)?;
                cx.factory().bool(present)
            }
            TableOp::Delete(key) => table.del(cx, key),
            TableOp::Keys => {
                let keys = table.keys(cx)?;
                let values = keys
                    .into_iter()
                    .map(|symbol| cx.factory().symbol(symbol))
                    .collect::<Result<Vec<_>>>()?;
                cx.factory().list(values)
            }
            TableOp::Entries => {
                let entries = table.entries(cx)?;
                cx.factory().table(entries)
            }
            TableOp::Len => {
                let len = table.len(cx)?;
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), len.to_string())
            }
            TableOp::Clear => {
                table.clear(cx)?;
                cx.factory().nil()
            }
            TableOp::Mkdir(name) => {
                let dir = current.object().as_dir().ok_or(Error::TypeMismatch {
                    expected: "directory table",
                    found: "non-directory",
                })?;
                let _ = dir.mkdir(cx, name.clone())?;
                cx.factory().string(Self::child_token(&path, &name))
            }
            TableOp::Opendir(name) => {
                let dir = current.object().as_dir().ok_or(Error::TypeMismatch {
                    expected: "directory table",
                    found: "non-directory",
                })?;
                match dir.opendir(cx, name.clone())? {
                    Some(_) => cx.factory().string(Self::child_token(&path, &name)),
                    None => cx.factory().nil(),
                }
            }
            TableOp::Rmdir(name) => {
                let dir = current.object().as_dir().ok_or(Error::TypeMismatch {
                    expected: "directory table",
                    found: "non-directory",
                })?;
                dir.rmdir(cx, name)
            }
            TableOp::IsDir(name) => {
                let dir = current.object().as_dir().ok_or(Error::TypeMismatch {
                    expected: "directory table",
                    found: "non-directory",
                })?;
                let is_dir = dir.is_dir(cx, name)?;
                cx.factory().bool(is_dir)
            }
        }?;
        Ok(Some(self.reply_value(cx, &frame, value, consistency)?))
    }
}

impl EvalSite for RemoteTableSite {
    fn site_kind(&self) -> &'static str {
        "remote-table"
    }

    fn address(&self) -> &ServerAddress {
        self.inner.address()
    }

    fn codecs(&self) -> &[Symbol] {
        self.inner.codecs()
    }

    fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
        if let Some(reply) = self.answer_table_request(cx, frame.clone())? {
            return Ok(reply);
        }
        self.inner.answer(cx, frame)
    }

    fn answer_with_timeout(
        &self,
        cx: &mut Cx,
        frame: ServerFrame,
        timeout: Option<std::time::Duration>,
    ) -> Result<ServerFrame> {
        if let Some(reply) = self.answer_table_request(cx, frame.clone())? {
            return Ok(reply);
        }
        self.inner.answer_with_timeout(cx, frame, timeout)
    }

    fn close_connection(&self, cx: &mut Cx) -> Result<()> {
        self.inner.close_connection(cx)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
