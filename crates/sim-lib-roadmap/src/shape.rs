use crate::{RoadmapValueKind, roadmap_value_from_expr};
use sim_kernel::{Cx, MatchScore, Result, Shape, ShapeDoc, ShapeMatch, Symbol, Value};

/// Shape for one native roadmap value kind.
#[derive(Clone, Copy, Debug)]
pub struct RoadmapValueShape {
    kind: Option<RoadmapValueKind>,
}
impl RoadmapValueShape {
    pub const fn any() -> Self {
        Self { kind: None }
    }
    pub const fn new(kind: RoadmapValueKind) -> Self {
        Self { kind: Some(kind) }
    }
}
impl Shape for RoadmapValueShape {
    fn symbol(&self) -> Option<Symbol> {
        Some(Symbol::qualified(
            "roadmap",
            self.kind.map_or("Value", RoadmapValueKind::wire_name),
        ))
    }
    fn check_value(&self, _cx: &mut Cx, value: Value) -> Result<ShapeMatch> {
        Ok(match value.object().downcast_ref::<crate::RoadmapValue>() {
            Some(v) if self.kind.is_none_or(|k| k == v.kind()) => {
                ShapeMatch::accept(MatchScore::exact(10))
            }
            _ => ShapeMatch::reject("expected admitted roadmap value"),
        })
    }
    fn check_expr(&self, _cx: &mut Cx, expr: &sim_kernel::Expr) -> Result<ShapeMatch> {
        Ok(match roadmap_value_from_expr(expr) {
            Ok(v) if self.kind.is_none_or(|k| k == v.kind()) => {
                ShapeMatch::accept(MatchScore::exact(10))
            }
            Ok(_) => ShapeMatch::reject("wrong roadmap value kind"),
            Err(e) => ShapeMatch::reject(e.to_string()),
        })
    }
    fn describe(&self, _cx: &mut Cx) -> Result<ShapeDoc> {
        Ok(ShapeDoc::new("bounded admitted roadmap value"))
    }
}

/// Shape for constructor argument and operation input/result lists.
#[derive(Clone, Copy, Debug, Default)]
pub struct RoadmapConstructorArgsShape;
impl Shape for RoadmapConstructorArgsShape {
    fn check_value(&self, cx: &mut Cx, value: Value) -> Result<ShapeMatch> {
        let expr = value.object().as_expr(cx)?;
        self.check_expr(cx, &expr)
    }
    fn check_expr(&self, _cx: &mut Cx, expr: &sim_kernel::Expr) -> Result<ShapeMatch> {
        Ok(match expr {
            sim_kernel::Expr::List(v) | sim_kernel::Expr::Vector(v)
                if v.len() == 1 && roadmap_value_from_expr(&v[0]).is_ok() =>
            {
                ShapeMatch::accept(MatchScore::exact(10))
            }
            _ => ShapeMatch::reject("expected one admitted roadmap expression"),
        })
    }
    fn describe(&self, _cx: &mut Cx) -> Result<ShapeDoc> {
        Ok(ShapeDoc::new("one bounded roadmap value argument"))
    }
}
