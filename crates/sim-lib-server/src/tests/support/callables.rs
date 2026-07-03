use sim_kernel::{Args, Callable, Expr, Object, Symbol, Value};

use super::{NEXT_TEST_VALUE_ID, Ordering};

#[derive(Clone)]
pub(crate) struct UntilValueFn {
    pub(crate) expected: &'static str,
}

impl Object for UntilValueFn {
    fn display(&self, _cx: &mut sim_kernel::Cx) -> sim_kernel::Result<String> {
        Ok("#<function test/until-value>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for UntilValueFn {
    fn class(&self, cx: &mut sim_kernel::Cx) -> sim_kernel::Result<sim_kernel::ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            sim_kernel::CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for UntilValueFn {
    fn call(&self, cx: &mut sim_kernel::Cx, args: Args) -> sim_kernel::Result<Value> {
        let Some(first) = args.values().first() else {
            return cx.factory().bool(false);
        };
        let Expr::String(text) = first.object().as_expr(cx)? else {
            return cx.factory().bool(false);
        };
        cx.factory().bool(text == self.expected)
    }
}

#[derive(Clone)]
pub(crate) struct YieldingSiteFn;

impl Object for YieldingSiteFn {
    fn display(&self, _cx: &mut sim_kernel::Cx) -> sim_kernel::Result<String> {
        Ok("#<function test/yield-site>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for YieldingSiteFn {
    fn class(&self, cx: &mut sim_kernel::Cx) -> sim_kernel::Result<sim_kernel::ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            sim_kernel::CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for YieldingSiteFn {
    fn call(&self, cx: &mut sim_kernel::Cx, args: Args) -> sim_kernel::Result<Value> {
        let coro = args.values()[0].clone();
        let frame = args.values()[1].clone();
        let id = NEXT_TEST_VALUE_ID.fetch_add(1, Ordering::Relaxed);
        let coro_symbol = Symbol::qualified("test", format!("yield-coro-{id}"));
        let frame_symbol = Symbol::qualified("test", format!("yield-frame-{id}"));
        cx.registry_mut()
            .register_value(coro_symbol.clone(), coro)?;
        cx.registry_mut()
            .register_value(frame_symbol.clone(), frame)?;
        cx.call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "yield"))?,
            vec![Expr::Symbol(coro_symbol), Expr::Symbol(frame_symbol)],
        )
    }
}

#[derive(Clone)]
pub(crate) struct RecordFn {
    pub(crate) seen: super::Arc<super::Mutex<Vec<String>>>,
}

impl Object for RecordFn {
    fn display(&self, _cx: &mut sim_kernel::Cx) -> sim_kernel::Result<String> {
        Ok("#<function test/record>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for RecordFn {
    fn class(&self, cx: &mut sim_kernel::Cx) -> sim_kernel::Result<sim_kernel::ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            sim_kernel::CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for RecordFn {
    fn call(&self, cx: &mut sim_kernel::Cx, args: Args) -> sim_kernel::Result<Value> {
        let Some(first) = args.values().first() else {
            return cx.factory().nil();
        };
        let Expr::String(text) = first.object().as_expr(cx)? else {
            return Err(sim_kernel::Error::TypeMismatch {
                expected: "string",
                found: "non-string",
            });
        };
        self.seen
            .lock()
            .expect("record fn mutex poisoned")
            .push(text.clone());
        cx.factory().string(text)
    }
}

#[derive(Clone)]
pub(crate) struct DecodeRecordFn;

impl Object for DecodeRecordFn {
    fn display(&self, _cx: &mut sim_kernel::Cx) -> sim_kernel::Result<String> {
        Ok("#<function test/decode-record>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for DecodeRecordFn {
    fn class(&self, cx: &mut sim_kernel::Cx) -> sim_kernel::Result<sim_kernel::ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            sim_kernel::CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for DecodeRecordFn {
    fn call(&self, cx: &mut sim_kernel::Cx, args: Args) -> sim_kernel::Result<Value> {
        let Some(first) = args.values().first() else {
            return Err(sim_kernel::Error::Eval(
                "decode-record expects an event string".to_owned(),
            ));
        };
        let Expr::String(text) = first.object().as_expr(cx)? else {
            return Err(sim_kernel::Error::TypeMismatch {
                expected: "string",
                found: "non-string",
            });
        };
        cx.factory().expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("test", "record"))),
            args: vec![Expr::String(text)],
        })
    }
}

#[derive(Clone)]
pub(crate) struct ConstantFn {
    pub(crate) value: &'static str,
}

impl Object for ConstantFn {
    fn display(&self, _cx: &mut sim_kernel::Cx) -> sim_kernel::Result<String> {
        Ok("#<function test/constant>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl sim_kernel::ObjectCompat for ConstantFn {
    fn class(&self, cx: &mut sim_kernel::Cx) -> sim_kernel::Result<sim_kernel::ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            sim_kernel::CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for ConstantFn {
    fn call(&self, cx: &mut sim_kernel::Cx, _args: Args) -> sim_kernel::Result<Value> {
        cx.factory().string(self.value.to_owned())
    }
}
