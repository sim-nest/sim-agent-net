use sim_kernel::{CapabilityName, Cx, Error, Expr, Result};

use crate::content::{field, json_content};
use crate::exec::{
    execute_surface_call, mcp_resources_read_capability, require_surface_capabilities,
};
use crate::uri::{
    McpResourceUriKind, not_found_error, optional_field, required_string_field, resource_uri_kind,
};
use crate::{McpSession, McpSurfaceCard, McpSurfaceRole, McpSurfaceSource, project_mcp_surface};

struct McpReadParams {
    uri: String,
}

impl McpReadParams {
    fn from_expr(expr: &Expr) -> Result<Self> {
        match expr {
            Expr::String(uri) => Ok(Self { uri: uri.clone() }),
            Expr::Map(fields) => Ok(Self {
                uri: required_string_field(fields, "uri")?,
            }),
            _ => Err(Error::TypeMismatch {
                expected: "resources/read params",
                found: "non-map",
            }),
        }
    }
}

pub(crate) fn list(cx: &mut Cx, session: &McpSession) -> Result<Expr> {
    let rows = resource_rows(cx, session)?
        .into_iter()
        .filter(|row| capabilities_allowed(&row.capabilities, &session.granted_capabilities))
        .map(resource_descriptor)
        .collect::<Vec<_>>();
    let mut rows = rows;
    rows.sort_by_key(resource_uri);
    Ok(Expr::Map(vec![field("resources", Expr::List(rows))]))
}

pub(crate) fn read(cx: &mut Cx, session: &McpSession, params: Expr) -> Result<Expr> {
    let params = McpReadParams::from_expr(&params)?;
    let row = resolve_resource_row(cx, session, &params.uri)?;
    match row.source {
        McpSurfaceSource::NativeCard => read_native_resource(&row, session),
        McpSurfaceSource::SkillCard => read_skill_resource(cx, session, &row),
    }
}

fn read_native_resource(row: &McpSurfaceCard, session: &McpSession) -> Result<Expr> {
    require_surface_capabilities(session, row, mcp_resources_read_capability())?;
    Ok(read_result(
        row.uri.as_deref().unwrap_or_default(),
        json_content(Expr::Map(vec![
            field("kind", Expr::String("sim-resource".to_owned())),
            field("uri", Expr::String(row.uri.clone().unwrap_or_default())),
            field("name", Expr::String(row.name.clone())),
            field("description", Expr::String(row.description.clone())),
        ])),
    ))
}

fn read_skill_resource(cx: &mut Cx, session: &McpSession, row: &McpSurfaceCard) -> Result<Expr> {
    let result = execute_surface_call(
        cx,
        session,
        row,
        mcp_resources_read_capability(),
        Vec::new(),
        "MCP resource",
    )?;
    Ok(Expr::Map(vec![field(
        "contents",
        Expr::List(
            result
                .content
                .into_iter()
                .map(|content| content_with_uri(row.uri.as_deref().unwrap_or_default(), content))
                .collect(),
        ),
    )]))
}

fn read_result(uri: &str, content: Expr) -> Expr {
    Expr::Map(vec![field(
        "contents",
        Expr::List(vec![content_with_uri(uri, content)]),
    )])
}

fn content_with_uri(uri: &str, content: Expr) -> Expr {
    match content {
        Expr::Map(mut fields) => {
            fields.insert(0, field("uri", Expr::String(uri.to_owned())));
            Expr::Map(fields)
        }
        other => Expr::Map(vec![
            field("uri", Expr::String(uri.to_owned())),
            field("type", Expr::String("json".to_owned())),
            field("json", other),
        ]),
    }
}

fn resolve_resource_row(cx: &mut Cx, session: &McpSession, uri: &str) -> Result<McpSurfaceCard> {
    resource_rows(cx, session)?
        .into_iter()
        .find(|row| row.uri.as_deref() == Some(uri))
        .ok_or_else(|| not_found_error("resource", uri))
}

fn resource_rows(cx: &mut Cx, session: &McpSession) -> Result<Vec<McpSurfaceCard>> {
    Ok(
        project_mcp_surface(cx, &session.native_cards, &session.profile)?
            .into_iter()
            .filter(|row| row.role == McpSurfaceRole::Resource)
            .filter(resource_uri_allowed)
            .collect(),
    )
}

fn resource_uri_allowed(row: &McpSurfaceCard) -> bool {
    let Some(uri) = row.uri.as_deref() else {
        return false;
    };
    match (&row.source, resource_uri_kind(uri)) {
        (McpSurfaceSource::NativeCard, McpResourceUriKind::Sim) => native_sim_uri_allowed(uri),
        (McpSurfaceSource::SkillCard, McpResourceUriKind::Skill) => true,
        _ => false,
    }
}

fn native_sim_uri_allowed(uri: &str) -> bool {
    ["sim://browse/", "sim://help/", "sim://test/", "sim://card/"]
        .iter()
        .any(|prefix| uri.starts_with(prefix))
}

fn resource_descriptor(row: McpSurfaceCard) -> Expr {
    Expr::Map(vec![
        field("uri", Expr::String(row.uri.unwrap_or_default())),
        field("name", Expr::String(row.name)),
        field("description", Expr::String(row.description)),
    ])
}

fn capabilities_allowed(required: &[CapabilityName], granted: &[CapabilityName]) -> bool {
    required
        .iter()
        .all(|capability| granted.iter().any(|granted| granted == capability))
}

fn resource_uri(expr: &Expr) -> String {
    let Expr::Map(fields) = expr else {
        return String::new();
    };
    optional_field(fields, "uri")
        .and_then(|expr| match expr {
            Expr::String(uri) => Some(uri.clone()),
            _ => None,
        })
        .unwrap_or_default()
}
