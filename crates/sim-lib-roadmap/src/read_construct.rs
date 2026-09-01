use crate::{RoadmapConstructorArgsShape, roadmap_value, roadmap_value_from_expr};
use sim_kernel::{
    Cx, Error, Object, ObjectCompat, ReadConstructor, Result, ShapeRef, Symbol, Value,
};
use std::any::Any;

/// Public constructor using the exact same strict inverse as direct decoding.
#[derive(Clone, Copy, Debug, Default)]
pub struct RoadmapValueReadConstructor;
impl Object for RoadmapValueReadConstructor {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok("#<read-constructor roadmap/Value>".into())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl ObjectCompat for RoadmapValueReadConstructor {
    fn as_read_constructor(&self) -> Option<&dyn ReadConstructor> {
        Some(self)
    }
}
impl ReadConstructor for RoadmapValueReadConstructor {
    fn symbol(&self) -> Symbol {
        Symbol::qualified("roadmap", "Value")
    }
    fn args_shape(&self, cx: &mut Cx) -> Result<ShapeRef> {
        cx.factory()
            .opaque(std::sync::Arc::new(RoadmapConstructorArgsShape))
    }
    fn construct_read(&self, cx: &mut Cx, args: Vec<Value>) -> Result<Value> {
        let [arg] = args.as_slice() else {
            return Err(Error::Eval("roadmap/Value expects one expression".into()));
        };
        let expr = arg.object().as_expr(cx)?;
        roadmap_value(cx, roadmap_value_from_expr(&expr)?)
    }
}
