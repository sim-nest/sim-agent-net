use sim_kernel::{CapabilityName, Cx, Error, Expr, Result};

use crate::content::{field, json_content};
use crate::exec::{execute_surface_call, mcp_prompts_get_capability, require_surface_capabilities};
use crate::uri::{not_found_error, optional_field, required_string_field};
use crate::{McpSession, McpSurfaceCard, McpSurfaceRole, McpSurfaceSource, project_mcp_surface};

struct McpGetPromptParams {
    name: String,
    arguments: Vec<Expr>,
}

impl McpGetPromptParams {
    fn from_expr(expr: &Expr) -> Result<Self> {
        match expr {
            Expr::String(name) => Ok(Self {
                name: name.clone(),
                arguments: Vec::new(),
            }),
            Expr::Map(fields) => Ok(Self {
                name: required_string_field(fields, "name")?,
                arguments: optional_arguments(fields)?,
            }),
            _ => Err(Error::TypeMismatch {
                expected: "prompts/get params",
                found: "non-map",
            }),
        }
    }
}

pub(crate) fn list(cx: &mut Cx, session: &McpSession) -> Result<Expr> {
    let rows = prompt_rows(cx, session)?
        .into_iter()
        .filter(|row| capabilities_allowed(&row.capabilities, &session.granted_capabilities))
        .map(|row| prompt_descriptor(cx, row))
        .collect::<Result<Vec<_>>>()?;
    let mut rows = rows;
    rows.sort_by_key(prompt_name);
    Ok(Expr::Map(vec![field("prompts", Expr::List(rows))]))
}

pub(crate) fn get(cx: &mut Cx, session: &McpSession, params: Expr) -> Result<Expr> {
    let params = McpGetPromptParams::from_expr(&params)?;
    let row = resolve_prompt_row(cx, session, &params.name)?;
    match row.source {
        McpSurfaceSource::NativeCard => get_native_prompt(&row, session),
        McpSurfaceSource::SkillCard => get_skill_prompt(cx, session, &row, params.arguments),
    }
}

fn get_native_prompt(row: &McpSurfaceCard, session: &McpSession) -> Result<Expr> {
    require_surface_capabilities(session, row, mcp_prompts_get_capability())?;
    Ok(prompt_result(
        row,
        vec![json_content(Expr::Map(vec![
            field("kind", Expr::String("sim-prompt".to_owned())),
            field("name", Expr::String(row.name.clone())),
            field("description", Expr::String(row.description.clone())),
        ]))],
        false,
    ))
}

fn get_skill_prompt(
    cx: &mut Cx,
    session: &McpSession,
    row: &McpSurfaceCard,
    arguments: Vec<Expr>,
) -> Result<Expr> {
    let result = execute_surface_call(
        cx,
        session,
        row,
        mcp_prompts_get_capability(),
        arguments,
        "MCP prompt",
    )?;
    Ok(prompt_result(row, result.content, result.is_error))
}

fn prompt_result(row: &McpSurfaceCard, content: Vec<Expr>, is_error: bool) -> Expr {
    let mut fields = vec![
        field("description", Expr::String(row.description.clone())),
        field(
            "messages",
            Expr::List(content.into_iter().map(prompt_message).collect()),
        ),
    ];
    if is_error {
        fields.push(field("isError", Expr::Bool(true)));
    }
    Expr::Map(fields)
}

fn prompt_message(content: Expr) -> Expr {
    Expr::Map(vec![
        field("role", Expr::String("user".to_owned())),
        field("content", content),
    ])
}

fn resolve_prompt_row(cx: &mut Cx, session: &McpSession, name: &str) -> Result<McpSurfaceCard> {
    prompt_rows(cx, session)?
        .into_iter()
        .find(|row| row.name == name)
        .ok_or_else(|| not_found_error("prompt", name))
}

fn prompt_rows(cx: &mut Cx, session: &McpSession) -> Result<Vec<McpSurfaceCard>> {
    Ok(
        project_mcp_surface(cx, &session.native_cards, &session.profile)?
            .into_iter()
            .filter(|row| row.role == McpSurfaceRole::Prompt)
            .collect(),
    )
}

fn prompt_descriptor(cx: &mut Cx, row: McpSurfaceCard) -> Result<Expr> {
    let mut fields = vec![
        field("name", Expr::String(row.name)),
        field("description", Expr::String(row.description)),
    ];
    if row.input_shape.is_some() {
        fields.push(field(
            "inputSchema",
            crate::shape_to_json_schema(cx, row.input_shape.as_ref())?,
        ));
    }
    Ok(Expr::Map(fields))
}

fn optional_arguments(fields: &[(Expr, Expr)]) -> Result<Vec<Expr>> {
    match optional_field(fields, "arguments") {
        Some(Expr::List(items)) => Ok(items.clone()),
        Some(Expr::Nil) | None => Ok(Vec::new()),
        Some(Expr::Map(fields)) => Ok(vec![Expr::Map(fields.clone())]),
        Some(_) => Err(Error::TypeMismatch {
            expected: "argument list, map, or nil",
            found: "invalid arguments",
        }),
    }
}

fn capabilities_allowed(required: &[CapabilityName], granted: &[CapabilityName]) -> bool {
    required
        .iter()
        .all(|capability| granted.iter().any(|granted| granted == capability))
}

fn prompt_name(expr: &Expr) -> String {
    let Expr::Map(fields) = expr else {
        return String::new();
    };
    optional_field(fields, "name")
        .and_then(|expr| match expr {
            Expr::String(name) => Some(name.clone()),
            _ => None,
        })
        .unwrap_or_default()
}
