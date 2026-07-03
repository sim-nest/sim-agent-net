//! CLI entrypoint claim for the server lib.

use std::sync::Arc;

use sim_kernel::{
    Args, CORE_FUNCTION_CLASS_ID, Callable, ClassRef, Cx, Error, Export, Expr, Linker, LoadCx,
    Object, ObjectCompat, Result, Symbol, Value,
};

const SERVER_CLI_VERB: &str = "server";

fn cli_main_symbol() -> Symbol {
    Symbol::qualified("cli", "main/server")
}

pub(crate) fn server_cli_exports() -> Vec<Export> {
    vec![Export::Function {
        symbol: cli_main_symbol(),
        function_id: None,
    }]
}

pub(crate) fn register_server_cli(cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
    linker.function_value(
        cli_main_symbol(),
        cx.factory().opaque(Arc::new(ServerCliEntrypoint))?,
    )?;
    Ok(())
}

#[derive(Clone)]
struct ServerCliEntrypoint;

impl Object for ServerCliEntrypoint {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<function cli/main/server>".to_owned())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for ServerCliEntrypoint {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }

    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for ServerCliEntrypoint {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        verify_cli_envelope(cx, &args, SERVER_CLI_VERB)?;
        cx.factory().bool(true)
    }
}

fn verify_cli_envelope(cx: &mut Cx, args: &Args, verb: &str) -> Result<()> {
    let envelope = args
        .values()
        .first()
        .ok_or_else(|| Error::Eval(format!("cli/main/{verb} expects a CLI envelope")))?;
    let envelope_verb = envelope_string_field(cx, envelope, "verb")?;
    if envelope_verb != verb {
        return Err(Error::Eval(format!(
            "cli/main/{verb} received verb {envelope_verb}"
        )));
    }
    let payload_args = envelope_args(cx, envelope)?;
    if payload_args.first().map(String::as_str) != Some(verb) {
        return Err(Error::Eval(format!(
            "cli/main/{verb} expects the first payload argument to be {verb}"
        )));
    }
    Ok(())
}

fn envelope_string_field(cx: &mut Cx, envelope: &Value, field: &str) -> Result<String> {
    let Some(table) = envelope.object().as_table_impl() else {
        return Err(Error::Eval("CLI envelope is not a table".to_owned()));
    };
    match table.get(cx, Symbol::new(field))?.object().as_expr(cx)? {
        Expr::String(text) => Ok(text),
        Expr::Nil => Err(Error::Eval(format!("CLI envelope field {field} is nil"))),
        other => Err(Error::Eval(format!(
            "CLI envelope field {field} is not a string: {other:?}"
        ))),
    }
}

fn envelope_args(cx: &mut Cx, envelope: &Value) -> Result<Vec<String>> {
    let Some(table) = envelope.object().as_table_impl() else {
        return Err(Error::Eval("CLI envelope is not a table".to_owned()));
    };
    let value = table.get(cx, Symbol::new("args"))?;
    let Some(list) = value.object().as_list() else {
        return Err(Error::Eval(
            "CLI envelope field args is not a list".to_owned(),
        ));
    };
    list.to_vec(cx, Some(64))?
        .into_iter()
        .map(|value| match value.object().as_expr(cx)? {
            Expr::String(text) => Ok(text),
            other => Err(Error::Eval(format!(
                "CLI payload argument is not a string: {other:?}"
            ))),
        })
        .collect()
}
