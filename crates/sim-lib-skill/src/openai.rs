use serde_json::{Map, Value as JsonValue};
use sim_codec_json::{JsonProjectionMode, project_json_to_expr};
use sim_kernel::{Args, Cx, Error, Expr, Result, Shape, Symbol, Value};

use crate::registry::card_from_value;
use crate::{SkillCard, SkillRole};

pub(crate) fn openai_tool(cx: &mut Cx, args: Args) -> Result<Value> {
    let card = card_arg(cx, args)?;
    let descriptor = skill_openai_tool_descriptor(cx, &card)?;
    cx.factory().expr(project_json_to_expr(
        &descriptor,
        JsonProjectionMode::UntaggedInterop,
    ))
}

pub(crate) fn openai_tools(cx: &mut Cx, args: Args) -> Result<Value> {
    let cards = selected_cards(cx, args)?;
    let descriptors = cards
        .iter()
        .map(|card| skill_openai_tool_descriptor(cx, card))
        .collect::<Result<Vec<_>>>()?;
    let values = descriptors
        .iter()
        .map(|descriptor| {
            cx.factory().expr(project_json_to_expr(
                descriptor,
                JsonProjectionMode::UntaggedInterop,
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    cx.factory().list(values)
}

/// Builds the OpenAI-compatible tool descriptor JSON for `card`.
///
/// Errors if the card is not marked with the [`SkillRole::Tool`] role. The
/// descriptor name is derived from the card's symbol, and the parameter schema
/// is derived from the card's input shape.
pub fn skill_openai_tool_descriptor(cx: &mut Cx, card: &SkillCard) -> Result<JsonValue> {
    ensure_tool_role(card)?;
    let name = {
        let mangled = sim_lib_surface_card::external_name(
            &card.symbol,
            sim_lib_surface_card::ExternalNamePolicy::OpenAiTool,
        );
        if mangled.is_empty() {
            "skill".to_owned()
        } else {
            mangled
        }
    };
    let parameters = parameters_for_shape(cx, &card.input_shape)?;
    let tool = sim_lib_openai_server::OpenAiTool::from_callable(
        cx,
        name,
        card.symbol.clone(),
        card.description.clone(),
        parameters,
        card.capabilities.clone(),
    )?;
    let descriptor = tool.descriptor_json();
    sim_lib_openai_server::OpenAiTool::from_openai_descriptor(&descriptor)?;
    Ok(descriptor)
}

/// Inserts each card in `cards` into the OpenAI tool `registry` as a tool.
pub fn insert_skill_openai_tools(
    cx: &mut Cx,
    registry: &mut sim_lib_openai_server::OpenAiToolRegistry,
    cards: &[SkillCard],
) -> Result<()> {
    for card in cards {
        let descriptor = skill_openai_tool_descriptor(cx, card)?;
        registry.insert(sim_lib_openai_server::OpenAiTool::from_openai_descriptor(
            &descriptor,
        )?)?;
    }
    Ok(())
}

/// Returns the symbol for the `skill/openai-tool` operation.
pub fn skill_openai_tool_symbol() -> Symbol {
    Symbol::qualified("skill", "openai-tool")
}

/// Returns the symbol for the `skill/openai-tools` operation.
pub fn skill_openai_tools_symbol() -> Symbol {
    Symbol::qualified("skill", "openai-tools")
}

fn selected_cards(cx: &mut Cx, args: Args) -> Result<Vec<SkillCard>> {
    let values = args.into_vec();
    if values.is_empty() {
        let cards = crate::skill_registry(cx)?
            .cards()?
            .into_iter()
            .filter(|card| card.roles.contains(&SkillRole::Tool))
            .collect::<Vec<_>>();
        return Ok(cards);
    }
    values
        .into_iter()
        .map(|value| card_from_target(cx, value))
        .collect()
}

fn card_arg(cx: &mut Cx, args: Args) -> Result<SkillCard> {
    let mut values = args.into_vec();
    if values.len() != 1 {
        return Err(Error::Eval(
            "skill/openai-tool expects one skill id, symbol, or SkillCard value".to_owned(),
        ));
    }
    card_from_target(cx, values.remove(0))
}

fn card_from_target(cx: &mut Cx, value: Value) -> Result<SkillCard> {
    if value.object().downcast_ref::<SkillCard>().is_some() {
        return card_from_value(cx, &value);
    }
    match value.object().as_expr(cx)? {
        Expr::Map(_) => card_from_value(cx, &value),
        Expr::String(id) => crate::skill_registry(cx)?
            .card_by_id(&id)?
            .ok_or_else(|| Error::Eval(format!("unknown skill {id}"))),
        Expr::Symbol(symbol) => crate::skill_registry(cx)?
            .card_by_symbol(&symbol)?
            .ok_or_else(|| Error::Eval(format!("unknown skill {symbol}"))),
        _ => Err(Error::TypeMismatch {
            expected: "skill id, symbol, or SkillCard",
            found: "invalid target",
        }),
    }
}

fn ensure_tool_role(card: &SkillCard) -> Result<()> {
    if card.roles.contains(&SkillRole::Tool) {
        Ok(())
    } else {
        Err(Error::Eval(format!(
            "skill {} is not marked with tool role",
            card.id
        )))
    }
}

fn parameters_for_shape(cx: &mut Cx, shape: &Value) -> Result<JsonValue> {
    let Some(shape) = shape.object().as_shape() else {
        return Ok(empty_parameters());
    };
    let doc = shape.describe(cx)?;
    if doc.name == "list shape" {
        return Ok(list_parameters_from_doc(&doc.details));
    }
    let mut properties = Map::new();
    properties.insert("value".to_owned(), schema_for_shape(cx, shape)?);
    Ok(parameters_object(properties, vec!["value".to_owned()]))
}

fn list_parameters_from_doc(details: &[String]) -> JsonValue {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (index, detail) in details
        .iter()
        .filter(|detail| *detail != "rest")
        .enumerate()
    {
        let name = format!("arg{index}");
        properties.insert(name.clone(), schema_for_doc_name(detail));
        required.push(name);
    }
    parameters_object(properties, required)
}

fn parameters_object(properties: Map<String, JsonValue>, required: Vec<String>) -> JsonValue {
    let mut object = Map::new();
    object.insert("type".to_owned(), JsonValue::String("object".to_owned()));
    object.insert("properties".to_owned(), JsonValue::Object(properties));
    object.insert(
        "required".to_owned(),
        JsonValue::Array(required.into_iter().map(JsonValue::String).collect()),
    );
    JsonValue::Object(object)
}

fn empty_parameters() -> JsonValue {
    parameters_object(Map::new(), Vec::new())
}

fn schema_for_shape(cx: &mut Cx, shape: &dyn Shape) -> Result<JsonValue> {
    let kind = shape
        .symbol()
        .and_then(|symbol| json_type_for_symbol(&symbol))
        .or_else(|| json_type_for_doc_name(&shape.describe(cx).ok()?.name));
    Ok(match kind {
        Some(kind) => serde_json::json!({ "type": kind }),
        None => JsonValue::Object(Map::new()),
    })
}

fn json_type_for_symbol(symbol: &Symbol) -> Option<&'static str> {
    match (symbol.namespace.as_deref(), symbol.name.as_ref()) {
        (Some("core"), "Number") => Some("number"),
        (Some("core"), "String") => Some("string"),
        (Some("core"), "Bool") => Some("boolean"),
        _ => None,
    }
}

fn json_type_for_doc_name(name: &str) -> Option<&'static str> {
    if name.contains("number") {
        Some("number")
    } else if name.contains("string") {
        Some("string")
    } else if name.contains("bool") {
        Some("boolean")
    } else {
        None
    }
}

fn schema_for_doc_name(name: &str) -> JsonValue {
    match json_type_for_doc_name(name) {
        Some(kind) => serde_json::json!({ "type": kind }),
        None => JsonValue::Object(Map::new()),
    }
}

#[cfg(test)]
mod tests;
