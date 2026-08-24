use crate::{RoadmapValue, roadmap_card, roadmap_value_to_expr};
use sim_kernel::{
    Cx, Expr, Object, ObjectCompat, ObjectEncode, ObjectEncoding, Result, Symbol, Value,
};
use std::{any::Any, sync::Arc};

impl Object for RoadmapValue {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!(
            "#<roadmap/{} {}>",
            self.kind().wire_name(),
            self.semantic_id()
        ))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl ObjectCompat for RoadmapValue {
    fn as_expr(&self, _cx: &mut Cx) -> Result<Expr> {
        Ok(roadmap_value_to_expr(self))
    }
    fn as_table(&self, cx: &mut Cx) -> Result<Value> {
        roadmap_card(cx, self)
    }
    fn as_object_encoder(&self) -> Option<&dyn ObjectEncode> {
        Some(self)
    }
}
impl ObjectEncode for RoadmapValue {
    fn object_encoding(&self, _cx: &mut Cx) -> Result<ObjectEncoding> {
        Ok(ObjectEncoding::Constructor {
            class: Symbol::qualified("roadmap", "Value"),
            args: vec![roadmap_value_to_expr(self)],
        })
    }
}
pub fn roadmap_value(cx: &mut Cx, value: RoadmapValue) -> Result<Value> {
    cx.factory().opaque(Arc::new(value))
}
