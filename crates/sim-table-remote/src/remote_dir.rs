use std::{sync::Arc, time::Duration};

use sim_kernel::{
    Consistency, Cx, Error, EvalMode, EvalRequest, Expr, Object, ObjectEncode, ObjectEncoding,
    Result, Symbol, Table, TableCompareExchange, TableExpected, TableObserved, TableReplacement,
    Value, capability::eval_remote_capability, id::CORE_TABLE_CLASS_ID, object::ClassRef,
    table::Dir,
};
use sim_lib_server::{EvalSite, eval_reply_from_frame, server_frame_from_request};
use sim_table_core::{CompareExpected, CompareReplacement, TableOp, encode_table_op};

use crate::{citizen::remote_dir_class_symbol, table_remote_capability};

/// SIM table directory whose rows live on a remote [`EvalSite`].
#[derive(Clone)]
pub struct RemoteDir {
    site: Arc<dyn EvalSite>,
    path: Vec<Symbol>,
    codec: Symbol,
}

impl RemoteDir {
    /// Creates a directory rooted at `site` using `codec`.
    ///
    /// Returns an error when `codec` is not among the codecs the site supports.
    pub fn new(site: Arc<dyn EvalSite>, codec: Symbol) -> Result<Self> {
        if !site.codecs().iter().any(|candidate| candidate == &codec) {
            return Err(Error::Eval(format!(
                "table/remote: codec {codec} is not supported by site {}",
                site.site_kind()
            )));
        }
        Ok(Self {
            site,
            path: Vec::new(),
            codec,
        })
    }

    fn with_path(&self, path: Vec<Symbol>) -> Self {
        Self {
            site: self.site.clone(),
            path,
            codec: self.codec.clone(),
        }
    }

    fn path_expr(&self) -> Expr {
        Expr::List(self.path.iter().cloned().map(Expr::Symbol).collect())
    }

    fn descriptor_path(&self) -> Vec<String> {
        self.path
            .iter()
            .map(|segment| segment.name.to_string())
            .collect()
    }

    fn remote_request(&self, op: &TableOp) -> EvalRequest {
        // The shared codec produces the `table/<wire>` operator and the op's own
        // args; the path is transport context prepended ahead of them, leaving
        // the on-wire Call byte-identical to the hand-built form.
        let Expr::Call { operator, args } = encode_table_op(op) else {
            unreachable!("encode_table_op always yields a Call");
        };
        let mut call_args = Vec::with_capacity(args.len() + 1);
        call_args.push(self.path_expr());
        call_args.extend(args);
        EvalRequest {
            expr: Expr::Call {
                operator,
                args: call_args,
            },
            mode: EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            required_capabilities: Vec::new(),
            deadline: Some(Duration::from_secs(5)),
            consistency: Consistency::RemoteOnly,
            trace: false,
        }
    }

    fn call(&self, cx: &mut Cx, op: &TableOp) -> Result<Value> {
        cx.require(&eval_remote_capability())?;
        let frame = server_frame_from_request(cx, &self.codec, self.remote_request(op))?;
        let reply = self.site.answer(cx, frame)?;
        Ok(eval_reply_from_frame(cx, &reply)?.value)
    }
}

impl Object for RemoteDir {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        if self.path.is_empty() {
            Ok(format!("table/remote[{}:/]", self.site.site_kind()))
        } else {
            let suffix = self
                .path
                .iter()
                .map(|segment| segment.to_string())
                .collect::<Vec<_>>()
                .join("/");
            Ok(format!("table/remote[{}:/{suffix}]", self.site.site_kind()))
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for RemoteDir {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        let symbol = remote_dir_class_symbol();
        if let Some(value) = cx.registry().class_by_symbol(&symbol) {
            return Ok(value.clone());
        }
        let symbol = Symbol::qualified("core", "Table");
        if let Some(value) = cx.registry().class_by_symbol(&symbol) {
            return Ok(value.clone());
        }
        cx.factory().class_stub(CORE_TABLE_CLASS_ID, symbol)
    }
    fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
        self.as_table_expr(cx)
    }
    fn truth(&self, cx: &mut Cx) -> Result<bool> {
        Ok(!self.is_empty(cx)?)
    }
    fn as_table_impl(&self) -> Option<&dyn Table> {
        Some(self)
    }
    fn as_dir(&self) -> Option<&dyn Dir> {
        Some(self)
    }
    fn as_object_encoder(&self) -> Option<&dyn ObjectEncode> {
        Some(self)
    }
}

impl ObjectEncode for RemoteDir {
    fn object_encoding(&self, _cx: &mut Cx) -> Result<ObjectEncoding> {
        Ok(ObjectEncoding::Constructor {
            class: remote_dir_class_symbol(),
            args: vec![
                Expr::Symbol(Symbol::new("v0")),
                Expr::String(self.site.site_kind().to_owned()),
                Expr::Symbol(self.codec.clone()),
                sim_table_core::citizen_fields::path_segments::encode(&self.descriptor_path()),
            ],
        })
    }
}

impl sim_citizen::Citizen for RemoteDir {
    fn citizen_symbol() -> Symbol {
        remote_dir_class_symbol()
    }

    fn citizen_version() -> u32 {
        0
    }

    fn citizen_arity() -> usize {
        3
    }

    fn citizen_fields() -> &'static [&'static str] {
        &["site_kind", "codec", "path"]
    }
}

impl Table for RemoteDir {
    fn backend_symbol(&self) -> Symbol {
        Symbol::qualified("table", "remote")
    }

    fn get(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        self.call(cx, &TableOp::Get(key))
    }

    fn set(&self, cx: &mut Cx, key: Symbol, value: Value) -> Result<()> {
        let expr = value.object().as_expr(cx)?;
        self.call(cx, &TableOp::Set(key, expr)).map(|_| ())
    }

    fn compare_exchange(
        &self,
        cx: &mut Cx,
        key: Symbol,
        expected: TableExpected,
        replacement: TableReplacement,
    ) -> Result<TableCompareExchange> {
        let expected = match expected {
            TableExpected::Absent => CompareExpected::Absent,
            TableExpected::Value(expr) => CompareExpected::Value(expr),
        };
        let replacement = match replacement {
            TableReplacement::Delete => CompareReplacement::Delete,
            TableReplacement::Value(value) => {
                CompareReplacement::Value(value.object().as_expr(cx)?)
            }
        };
        let reply = self
            .call(cx, &TableOp::CompareExchange(key, expected, replacement))?
            .object()
            .as_expr(cx)?;
        let Expr::Call { operator, args } = reply else {
            return Err(Error::Eval("table/remote: malformed cas result".into()));
        };
        if operator.as_ref() != &Expr::Symbol(Symbol::qualified("table", "cas-result")) {
            return Err(Error::Eval("table/remote: malformed cas result".into()));
        }
        let [Expr::Bool(exchanged), observed] = args.as_slice() else {
            return Err(Error::Eval("table/remote: malformed cas result".into()));
        };
        let Expr::Call { operator, args } = observed else {
            return Err(Error::Eval(
                "table/remote: malformed cas observation".into(),
            ));
        };
        let observed = match (operator.as_ref(), args.as_slice()) {
            (Expr::Symbol(s), []) if *s == Symbol::qualified("table", "absent") => {
                TableObserved::Absent
            }
            (Expr::Symbol(s), [value]) if *s == Symbol::qualified("table", "value") => {
                TableObserved::Value(value.clone())
            }
            _ => {
                return Err(Error::Eval(
                    "table/remote: malformed cas observation".into(),
                ));
            }
        };
        Ok(TableCompareExchange {
            exchanged: *exchanged,
            observed,
        })
    }

    fn has(&self, cx: &mut Cx, key: Symbol) -> Result<bool> {
        self.call(cx, &TableOp::Has(key))?.object().truth(cx)
    }

    fn del(&self, cx: &mut Cx, key: Symbol) -> Result<Value> {
        self.call(cx, &TableOp::Delete(key))
    }

    fn keys(&self, cx: &mut Cx) -> Result<Vec<Symbol>> {
        let reply = self.call(cx, &TableOp::Keys)?;
        let list = reply.object().as_list().ok_or(Error::TypeMismatch {
            expected: "list",
            found: "non-list",
        })?;
        list.to_vec(cx, None)?
            .into_iter()
            .map(|value| match value.object().as_expr(cx)? {
                Expr::Symbol(symbol) => Ok(symbol),
                _ => Err(Error::TypeMismatch {
                    expected: "symbol",
                    found: "non-symbol",
                }),
            })
            .collect()
    }

    fn entries(&self, cx: &mut Cx) -> Result<Vec<(Symbol, Value)>> {
        self.call(cx, &TableOp::Entries)?
            .object()
            .as_table_impl()
            .ok_or(Error::TypeMismatch {
                expected: "table",
                found: "non-table",
            })?
            .entries(cx)
    }

    fn len(&self, cx: &mut Cx) -> Result<usize> {
        match self.call(cx, &TableOp::Len)?.object().as_expr(cx)? {
            Expr::Number(number) => number
                .canonical
                .parse::<usize>()
                .map_err(|err| Error::Eval(format!("table/remote: invalid len reply: {err}"))),
            Expr::String(text) => text
                .parse::<usize>()
                .map_err(|err| Error::Eval(format!("table/remote: invalid len reply: {err}"))),
            _ => Err(Error::TypeMismatch {
                expected: "number",
                found: "non-number",
            }),
        }
    }

    fn clear(&self, cx: &mut Cx) -> Result<()> {
        self.call(cx, &TableOp::Clear).map(|_| ())
    }
}

impl Dir for RemoteDir {
    fn mkdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        self.call(cx, &TableOp::Mkdir(name.clone()))?;
        let mut path = self.path.clone();
        path.push(name);
        cx.factory().opaque(Arc::new(self.with_path(path)))
    }

    fn opendir(&self, cx: &mut Cx, name: Symbol) -> Result<Option<Value>> {
        let reply = self.call(cx, &TableOp::Opendir(name.clone()))?;
        if matches!(reply.object().as_expr(cx)?, Expr::Nil) {
            return Ok(None);
        }
        let mut path = self.path.clone();
        path.push(name);
        Ok(Some(cx.factory().opaque(Arc::new(self.with_path(path)))?))
    }

    fn rmdir(&self, cx: &mut Cx, name: Symbol) -> Result<Value> {
        self.call(cx, &TableOp::Rmdir(name))
    }

    fn is_dir(&self, cx: &mut Cx, name: Symbol) -> Result<bool> {
        self.call(cx, &TableOp::IsDir(name))?.object().truth(cx)
    }
}

/// Builds a [`RemoteDir`] over `site`/`codec` as an opaque runtime [`Value`].
///
/// Requires the remote-table capability gate.
pub fn remote_dir_value(cx: &mut Cx, site: Arc<dyn EvalSite>, codec: Symbol) -> Result<Value> {
    cx.require(&table_remote_capability())?;
    cx.factory().opaque(Arc::new(RemoteDir::new(site, codec)?))
}
