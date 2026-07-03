#[cfg(feature = "stream")]
use sim_codec_mcp::McpEnvelope;
use sim_kernel::{CapabilityName, Cx, Expr, Result};

use crate::content::McpCallParams;
use crate::{
    McpSession, McpSurfaceCard, McpSurfaceRole, project_mcp_surface, shape_to_json_schema,
};

pub(crate) fn list(cx: &mut Cx, session: &McpSession) -> Result<Expr> {
    let rows = project_mcp_surface(cx, &session.native_cards, &session.profile)?;
    let mut descriptors = rows
        .into_iter()
        .filter(|row| row.role == McpSurfaceRole::Tool)
        .filter(|row| capabilities_allowed(&row.capabilities, &session.granted_capabilities))
        .map(|row| tool_descriptor(cx, row))
        .collect::<Result<Vec<_>>>()?;
    descriptors.sort_by_key(descriptor_name);
    Ok(Expr::Map(vec![field("tools", Expr::List(descriptors))]))
}

pub(crate) fn call(cx: &mut Cx, session: &McpSession, params: Expr) -> Result<Expr> {
    let params = McpCallParams::from_expr(&params)?;
    Ok(crate::exec::execute_tool_call(cx, session, params)?.to_expr())
}

#[cfg(feature = "stream")]
pub(crate) fn call_with_stream(
    cx: &mut Cx,
    session: &mut McpSession,
    params: Expr,
    progress_token: Option<Expr>,
) -> Result<(Expr, Vec<McpEnvelope>)> {
    let params = McpCallParams::from_expr(&params)?;
    let (result, notifications) =
        crate::exec::execute_tool_call_with_stream(cx, session, params, progress_token.as_ref())?;
    Ok((result.to_expr(), notifications))
}

fn tool_descriptor(cx: &mut Cx, row: McpSurfaceCard) -> Result<Expr> {
    let mut fields = vec![
        field("name", Expr::String(row.name)),
        field("description", Expr::String(row.description)),
        field(
            "inputSchema",
            shape_to_json_schema(cx, row.input_shape.as_ref())?,
        ),
    ];
    if let Some(output_shape) = row.output_shape.as_ref() {
        fields.push(field(
            "outputSchema",
            shape_to_json_schema(cx, Some(output_shape))?,
        ));
    }
    if !row.annotations.is_empty() {
        fields.push(field(
            "annotations",
            Expr::Map(
                row.annotations
                    .into_iter()
                    .map(|(key, value)| (Expr::Symbol(key), value))
                    .collect(),
            ),
        ));
    }
    Ok(Expr::Map(fields))
}

fn capabilities_allowed(required: &[CapabilityName], granted: &[CapabilityName]) -> bool {
    required
        .iter()
        .all(|capability| granted.iter().any(|granted| granted == capability))
}

fn descriptor_name(expr: &Expr) -> String {
    field_value(expr, "name")
        .and_then(|expr| match expr {
            Expr::String(name) => Some(name.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn field_value<'a>(expr: &'a Expr, name: &str) -> Option<&'a Expr> {
    let Expr::Map(fields) = expr else {
        return None;
    };
    fields.iter().find_map(|(key, value)| {
        let field_name = match key {
            Expr::Symbol(symbol) if symbol.namespace.is_none() => symbol.name.as_ref(),
            Expr::String(text) => text.as_str(),
            _ => return None,
        };
        (field_name == name).then_some(value)
    })
}

use sim_value::build::entry as field;
