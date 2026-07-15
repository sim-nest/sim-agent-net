use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::support::{eval_cx, install_agent_lib, install_test_codec};
use crate::{exec_capability, find_capability, fs_read_capability};
use sim_kernel::{
    Args, CORE_FUNCTION_CLASS_ID, Callable, ClassRef, Cx, Error, Expr, Object, Result, Symbol,
    Value,
};

#[test]
fn read_tool_works_under_default_ceiling() {
    let mut cx = core_tools_cx();
    grant_default_core_ceiling(&mut cx);
    let table = cx
        .factory()
        .table(vec![(
            Symbol::new("alpha"),
            cx.factory().string("one".to_owned()).unwrap(),
        )])
        .unwrap();

    let key = cx.factory().symbol(Symbol::new("alpha")).unwrap();
    let value = call_tool(&mut cx, "read", vec![table, key]).unwrap();

    assert_eq!(
        value.object().as_expr(&mut cx).unwrap(),
        Expr::String("one".to_owned())
    );
}

#[test]
fn exec_tool_refused_without_grant() {
    let mut cx = core_tools_cx();
    grant_default_core_ceiling(&mut cx);
    let calls = register_fake_exec(&mut cx);

    let args = exec_args(&mut cx);
    let denied = call_tool(&mut cx, "exec", args).unwrap_err();

    assert!(matches!(
        denied,
        Error::CapabilityDenied { capability } if capability == exec_capability()
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn exec_tool_works_when_granted() {
    let mut cx = core_tools_cx();
    grant_default_core_ceiling(&mut cx);
    cx.grant(exec_capability());
    let calls = register_fake_exec(&mut cx);

    let args = exec_args(&mut cx);
    let value = call_tool(&mut cx, "exec", args).unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        value.object().as_expr(&mut cx).unwrap(),
        Expr::String("exec-ok".to_owned())
    );
}

#[test]
fn model_descriptor_lists_tools_with_capabilities() {
    let mut cx = core_tools_cx();
    let tools = cx
        .call_function(
            &Symbol::qualified("agent", "tools"),
            Args::new(vec![
                cx.factory().symbol(Symbol::new(":category")).unwrap(),
                cx.factory().symbol(Symbol::new("io")).unwrap(),
            ]),
        )
        .unwrap();
    let Expr::List(items) = tools.object().as_expr(&mut cx).unwrap() else {
        panic!("agent/tools should return a list");
    };

    assert!(items.len() >= 7);
    assert_capabilities(&items, "read", &["fs/read"]);
    assert_capabilities(&items, "write", &["fs/write"]);
    assert_capabilities(&items, "list", &["fs/read"]);
    assert_capabilities(&items, "edit", &["fs/read", "edit"]);
    assert_capabilities(&items, "find", &["fs/read", "find"]);
    assert_capabilities(&items, "exec", &["exec"]);
    assert_capabilities(&items, "fetch", &["net/http"]);
}

fn core_tools_cx() -> Cx {
    let mut cx = eval_cx();
    install_test_codec(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx
}

fn grant_default_core_ceiling(cx: &mut Cx) {
    cx.grant(fs_read_capability());
    cx.grant(find_capability());
}

fn call_tool(cx: &mut Cx, name: &str, args: Vec<Value>) -> Result<Value> {
    let mut values = vec![cx.factory().symbol(Symbol::new(name)).unwrap()];
    values.extend(args);
    cx.call_function(&Symbol::qualified("agent", "call-tool"), Args::new(values))
}

fn exec_args(cx: &mut Cx) -> Vec<Value> {
    let argv = cx
        .factory()
        .list(vec![cx.factory().string("echo".to_owned()).unwrap()])
        .unwrap();
    let options = cx
        .factory()
        .table(vec![
            (
                Symbol::new("timeout-ms"),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "1000".to_owned())
                    .unwrap(),
            ),
            (
                Symbol::new("max-output-bytes"),
                cx.factory()
                    .number_literal(Symbol::qualified("numbers", "f64"), "128".to_owned())
                    .unwrap(),
            ),
        ])
        .unwrap();
    vec![argv, options]
}

fn register_fake_exec(cx: &mut Cx) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let function = cx
        .factory()
        .opaque(Arc::new(FakeExec {
            calls: calls.clone(),
        }))
        .unwrap();
    cx.registry_mut()
        .register_function_value(Symbol::new("exec"), function)
        .unwrap();
    calls
}

fn assert_capabilities(items: &[Expr], name: &str, expected: &[&str]) {
    let map = items
        .iter()
        .find_map(|item| match item {
            Expr::Map(entries)
                if entries.iter().any(|(key, value)| {
                    *key == Expr::Symbol(Symbol::new("name"))
                        && *value == Expr::Symbol(Symbol::new(name))
                }) =>
            {
                Some(entries)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {name} tool descriptor"));
    let Expr::List(capabilities) = map
        .iter()
        .find_map(|(key, value)| {
            (*key == Expr::Symbol(Symbol::new("capabilities"))).then_some(value)
        })
        .unwrap()
    else {
        panic!("{name} capabilities should be a list");
    };
    let actual = capabilities
        .iter()
        .map(|expr| match expr {
            Expr::String(text) => text.as_str(),
            _ => panic!("{name} capability should be a string"),
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

struct FakeExec {
    calls: Arc<AtomicUsize>,
}

impl Object for FakeExec {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<function exec>".to_owned())
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for FakeExec {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        cx.factory().class_stub(
            CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }

    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for FakeExec {
    fn call(&self, cx: &mut Cx, _args: Args) -> Result<Value> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        cx.factory().string("exec-ok".to_owned())
    }
}
