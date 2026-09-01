use crate::{ROADMAP_OPERATIONS, apply_operation, roadmap_value};
use sim_codec::{Input, decode_with_codec};
use sim_kernel::{Args, Callable, Cx, Error, Expr, Object, ObjectCompat, Result, Value};

/// Callable loaded `sim roadmap` entrypoint. Arguments are caller-supplied admitted expressions.
#[derive(Clone)]
pub struct RoadmapCommand;
impl Object for RoadmapCommand {
    fn display(&self, _: &mut Cx) -> Result<String> {
        Ok("cli/main/roadmap".into())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
impl ObjectCompat for RoadmapCommand {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}
impl Callable for RoadmapCommand {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        let envelope = args
            .values()
            .first()
            .ok_or_else(|| Error::Eval("missing roadmap envelope".into()))?;
        let table = envelope
            .object()
            .as_table_impl()
            .ok_or_else(|| Error::Eval("roadmap envelope is not a table".into()))?;
        let Expr::List(argv) = table
            .get(cx, sim_kernel::Symbol::new("args"))?
            .object()
            .as_expr(cx)?
        else {
            return Err(Error::Eval("roadmap args are not a list".into()));
        };
        let mut it = argv.into_iter().filter_map(|e| {
            if let Expr::String(s) = e {
                Some(s)
            } else {
                None
            }
        });
        let _roadmap = it.next();
        let op = it.next().unwrap_or_else(|| "validate".into());
        if !ROADMAP_OPERATIONS.contains(&op.as_str()) {
            return Err(Error::Eval(format!("unknown roadmap subcommand {op}")));
        }
        ensure_lisp_codec(cx)?;
        let values = it
            .map(|text| parse_caller_value(cx, &text))
            .collect::<Result<Vec<_>>>()?;
        roadmap_value(cx, apply_operation(&op, &values)?)
    }
}
fn ensure_lisp_codec(cx: &mut Cx) -> Result<()> {
    let symbol = sim_kernel::Symbol::qualified("codec", "lisp");
    if cx.registry().codec_by_symbol(&symbol).is_none() {
        let id = cx.registry_mut().fresh_codec_id();
        cx.load_lib(&sim_codec_lisp::LispCodecLib::new(id)?)?;
    }
    Ok(())
}

fn parse_caller_value(cx: &mut Cx, text: &str) -> Result<crate::RoadmapValue> {
    let expr = decode_with_codec(
        cx,
        &sim_kernel::Symbol::qualified("codec", "lisp"),
        Input::Text(text.to_owned()),
        sim_kernel::ReadPolicy::default(),
    )?;
    crate::roadmap_value_from_expr(&expr)
}
