use std::collections::BTreeMap;

use sim_kernel::{CapabilityName, Expr, Symbol, Value};

use crate::install_server_lib;

use super::{cx, quoted};

pub(crate) fn table_field(entries: &[(Expr, Expr)], key: &str) -> Option<Expr> {
    entries
        .iter()
        .find_map(|(entry_key, entry_value)| match entry_key {
            Expr::Symbol(symbol) if symbol.name.as_ref() == key => Some(entry_value.clone()),
            _ => None,
        })
}

pub(crate) fn normalized_reflect_table(
    cx: &mut sim_kernel::Cx,
    value: Value,
) -> BTreeMap<String, Expr> {
    let Expr::Map(entries) = value.object().as_expr(cx).unwrap() else {
        panic!("server/reflect should produce a table expression");
    };
    let mut map = BTreeMap::new();
    for (key, value) in entries {
        let Expr::Symbol(symbol) = key else {
            continue;
        };
        if matches!(symbol.name.as_ref(), "id" | "uptime") {
            continue;
        }
        map.insert(symbol.as_qualified_str(), value);
    }
    map
}

pub(crate) fn assert_trigger_source_requires_capability(source: Expr, capability: CapabilityName) {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();

    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "cap-server"), server)
        .unwrap();

    let err = cx
        .call_exprs(
            cx.resolve_function(&Symbol::qualified("server", "trigger"))
                .unwrap(),
            vec![
                Expr::Symbol(Symbol::qualified("test", "cap-server")),
                Expr::Symbol(Symbol::new(":source")),
                source,
                Expr::Symbol(Symbol::new(":decode")),
                quoted(Expr::Symbol(Symbol::new("lisp"))),
            ],
        )
        .unwrap_err();
    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability: denied }
            if denied == capability
    ));
}
