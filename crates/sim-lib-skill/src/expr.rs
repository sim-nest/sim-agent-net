use sim_kernel::{CapabilityName, Cx, Error, Expr, Result, ShapeRef, Symbol};
use sim_shape::{parse_shape_expr, shape_value};

use crate::{
    SkillCacheMode, SkillCard, SkillCassetteMode, SkillPolicy, SkillPrivacyPolicy, SkillRole,
};

impl SkillRole {
    /// Parses a role from a symbol or string [`Expr`], erroring on an unknown
    /// or non-role expression.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        let name = match expr {
            Expr::Symbol(symbol) => symbol.name.as_ref(),
            Expr::String(text) => text.as_str(),
            _ => {
                return Err(Error::TypeMismatch {
                    expected: "skill role",
                    found: "non-role",
                });
            }
        };
        match name {
            "tool" => Ok(Self::Tool),
            "model" => Ok(Self::Model),
            "resource" => Ok(Self::Resource),
            "prompt" => Ok(Self::Prompt),
            "memory" => Ok(Self::Memory),
            "retriever" => Ok(Self::Retriever),
            "judge" => Ok(Self::Judge),
            "router" => Ok(Self::Router),
            _ => Err(Error::Eval(format!("unknown skill role {name}"))),
        }
    }
}

impl SkillCard {
    /// Encodes the card as a `skill/card` map [`Expr`], the inverse of
    /// [`SkillCard::from_expr`].
    pub fn to_expr(&self, cx: &mut Cx) -> Result<Expr> {
        Ok(Expr::Map(vec![
            field("kind", Expr::Symbol(Symbol::qualified("skill", "card"))),
            field("id", Expr::String(self.id.clone())),
            field("symbol", Expr::Symbol(self.symbol.clone())),
            field(
                "aliases",
                Expr::List(self.aliases.iter().cloned().map(Expr::Symbol).collect()),
            ),
            field("origin", Expr::Symbol(self.origin.clone())),
            field("title", Expr::String(self.title.clone())),
            field("description", Expr::String(self.description.clone())),
            field("input-shape", self.input_shape.object().as_expr(cx)?),
            field("output-shape", self.output_shape.object().as_expr(cx)?),
            field(
                "roles",
                Expr::List(
                    self.roles
                        .iter()
                        .map(|role| Expr::Symbol(role.as_symbol()))
                        .collect(),
                ),
            ),
            field(
                "capabilities",
                Expr::List(
                    self.capabilities
                        .iter()
                        .map(|capability| Expr::String(capability.as_str().to_owned()))
                        .collect(),
                ),
            ),
            field("policy", policy_expr(&self.policy)),
            field(
                "transport",
                Expr::Map(vec![
                    field("id", Expr::String(self.transport_id.clone())),
                    field(
                        "kind",
                        Expr::Symbol(Symbol::new(self.transport_kind.clone())),
                    ),
                    field("operation", Expr::String(self.operation.clone())),
                ]),
            ),
        ]))
    }

    /// Decodes a card from a `skill/card` map [`Expr`], the inverse of
    /// [`SkillCard::to_expr`].
    ///
    /// Missing policy fields fall back to [`SkillPolicy::default`].
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        let fields = map_fields(expr, "SkillCard")?;
        expect_kind(fields)?;
        let id = required_string(fields, "id")?;
        let symbol = required_symbol(fields, "symbol")?;
        let aliases = optional_list(fields, "aliases")
            .unwrap_or(&[])
            .iter()
            .map(symbol_from_expr)
            .collect::<Result<Vec<_>>>()?;
        let origin = required_symbol(fields, "origin")?;
        let title = required_string(fields, "title")?;
        let description = required_string(fields, "description")?;
        let input_shape_expr = required_field(fields, "input-shape")?;
        let output_shape_expr = required_field(fields, "output-shape")?;
        let roles = optional_list(fields, "roles")
            .unwrap_or(&[])
            .iter()
            .map(SkillRole::from_expr)
            .collect::<Result<Vec<_>>>()?;
        let capabilities = optional_list(fields, "capabilities")
            .unwrap_or(&[])
            .iter()
            .map(capability_from_expr)
            .collect::<Result<Vec<_>>>()?;
        let policy = match required_field(fields, "policy") {
            Ok(expr) => policy_from_expr(expr)?,
            Err(_) => SkillPolicy::default(),
        };
        let transport = map_fields(required_field(fields, "transport")?, "SkillCard transport")?;

        Ok(Self {
            id: id.clone(),
            symbol: symbol.clone(),
            aliases,
            origin,
            title,
            description,
            input_shape: shape_ref(
                shape_symbol(
                    Symbol::qualified(symbol.to_string(), "args"),
                    input_shape_expr,
                ),
                input_shape_expr,
            )?,
            output_shape: shape_ref(
                shape_symbol(
                    Symbol::qualified(symbol.to_string(), "result"),
                    output_shape_expr,
                ),
                output_shape_expr,
            )?,
            roles,
            capabilities,
            policy,
            transport_id: required_string(transport, "id")?,
            transport_kind: transport_kind(transport)?,
            operation: required_string(transport, "operation")?,
        })
    }
}

use sim_value::build::entry as field;

fn shape_ref(symbol: Symbol, expr: &Expr) -> Result<ShapeRef> {
    let shape = parse_shape_expr(expr)?;
    Ok(shape_value(symbol, shape))
}

fn shape_symbol(default: Symbol, expr: &Expr) -> Symbol {
    match expr {
        Expr::Symbol(symbol) => symbol.clone(),
        _ => default,
    }
}

fn expect_kind(fields: &[(Expr, Expr)]) -> Result<()> {
    let kind = required_symbol(fields, "kind")?;
    if kind == Symbol::qualified("skill", "card") {
        Ok(())
    } else {
        Err(Error::TypeMismatch {
            expected: "skill/card",
            found: "other map",
        })
    }
}

use sim_value::access::map_entries as map_fields;

fn required_field<'a>(fields: &'a [(Expr, Expr)], name: &str) -> Result<&'a Expr> {
    sim_value::access::entry_field(fields, name)
        .ok_or_else(|| Error::Eval(format!("SkillCard is missing field {name}")))
}

fn required_string(fields: &[(Expr, Expr)], name: &str) -> Result<String> {
    match required_field(fields, name)? {
        Expr::String(value) => Ok(value.clone()),
        _ => Err(Error::TypeMismatch {
            expected: "string",
            found: "non-string",
        }),
    }
}

fn required_symbol(fields: &[(Expr, Expr)], name: &str) -> Result<Symbol> {
    symbol_from_expr(required_field(fields, name)?)
}

fn symbol_from_expr(expr: &Expr) -> Result<Symbol> {
    match expr {
        Expr::Symbol(symbol) => Ok(symbol.clone()),
        Expr::String(text) => Ok(parse_symbol_text(text)),
        _ => Err(Error::TypeMismatch {
            expected: "symbol",
            found: "non-symbol",
        }),
    }
}

fn optional_list<'a>(fields: &'a [(Expr, Expr)], name: &str) -> Option<&'a [Expr]> {
    match required_field(fields, name).ok()? {
        Expr::List(items) => Some(items),
        _ => None,
    }
}

fn capability_from_expr(expr: &Expr) -> Result<CapabilityName> {
    match expr {
        Expr::String(text) => Ok(CapabilityName::new(text.clone())),
        Expr::Symbol(symbol) if symbol.namespace.as_deref() == Some("capability") => {
            Ok(CapabilityName::new(symbol.name.to_string()))
        }
        Expr::Symbol(symbol) => Ok(CapabilityName::new(symbol.to_string())),
        _ => Err(Error::TypeMismatch {
            expected: "capability",
            found: "non-capability",
        }),
    }
}

fn policy_expr(policy: &SkillPolicy) -> Expr {
    let mut fields = vec![
        field("privacy", Expr::Symbol(policy.privacy.as_symbol())),
        field("cache", Expr::Symbol(policy.cache.as_symbol())),
        field("cassette", Expr::Symbol(policy.cassette.as_symbol())),
        field("idempotent", Expr::Bool(policy.idempotent)),
    ];
    if let Some(semantic_key) = &policy.semantic_key {
        fields.push(field("semantic-key", Expr::String(semantic_key.clone())));
    }
    Expr::Map(fields)
}

fn policy_from_expr(expr: &Expr) -> Result<SkillPolicy> {
    let fields = map_fields(expr, "SkillCard policy")?;
    Ok(SkillPolicy {
        privacy: optional_field(fields, "privacy")
            .map(privacy_from_expr)
            .transpose()?
            .unwrap_or(SkillPrivacyPolicy::NoRaw),
        cache: optional_field(fields, "cache")
            .map(cache_mode_from_expr)
            .transpose()?
            .unwrap_or(SkillCacheMode::Disabled),
        cassette: optional_field(fields, "cassette")
            .map(cassette_mode_from_expr)
            .transpose()?
            .unwrap_or(SkillCassetteMode::Disabled),
        idempotent: optional_field(fields, "idempotent")
            .map(bool_from_expr)
            .transpose()?
            .unwrap_or(false),
        semantic_key: optional_field(fields, "semantic-key")
            .map(stringish_from_expr)
            .transpose()?,
    })
}

fn privacy_from_expr(expr: &Expr) -> Result<SkillPrivacyPolicy> {
    match symbol_name(expr)?.as_str() {
        "metadata-only" => Ok(SkillPrivacyPolicy::MetadataOnly),
        "no-raw" => Ok(SkillPrivacyPolicy::NoRaw),
        "local-only" => Ok(SkillPrivacyPolicy::LocalOnly),
        "allow-raw" => Ok(SkillPrivacyPolicy::AllowRaw),
        other => Err(Error::Eval(format!("unknown skill privacy policy {other}"))),
    }
}

fn cache_mode_from_expr(expr: &Expr) -> Result<SkillCacheMode> {
    match symbol_name(expr)?.as_str() {
        "disabled" => Ok(SkillCacheMode::Disabled),
        "read-through" => Ok(SkillCacheMode::ReadThrough),
        "read-only" => Ok(SkillCacheMode::ReadOnly),
        "write-only" => Ok(SkillCacheMode::WriteOnly),
        "refresh" => Ok(SkillCacheMode::Refresh),
        other => Err(Error::Eval(format!("unknown skill cache mode {other}"))),
    }
}

fn cassette_mode_from_expr(expr: &Expr) -> Result<SkillCassetteMode> {
    match symbol_name(expr)?.as_str() {
        "disabled" => Ok(SkillCassetteMode::Disabled),
        "record-replay" => Ok(SkillCassetteMode::RecordReplay),
        "replay-only" => Ok(SkillCassetteMode::ReplayOnly),
        "record-only" => Ok(SkillCassetteMode::RecordOnly),
        other => Err(Error::Eval(format!("unknown skill cassette mode {other}"))),
    }
}

fn bool_from_expr(expr: &Expr) -> Result<bool> {
    match expr {
        Expr::Bool(value) => Ok(*value),
        _ => Err(Error::TypeMismatch {
            expected: "bool",
            found: "non-bool",
        }),
    }
}

fn symbol_name(expr: &Expr) -> Result<String> {
    match expr {
        Expr::Symbol(symbol) => Ok(symbol.name.to_string()),
        Expr::String(text) => Ok(text.clone()),
        _ => Err(Error::TypeMismatch {
            expected: "symbol or string",
            found: "invalid policy value",
        }),
    }
}

fn stringish_from_expr(expr: &Expr) -> Result<String> {
    match expr {
        Expr::String(text) => Ok(text.clone()),
        Expr::Symbol(symbol) => Ok(symbol.to_string()),
        _ => Err(Error::TypeMismatch {
            expected: "string",
            found: "non-string",
        }),
    }
}

fn optional_field<'a>(fields: &'a [(Expr, Expr)], name: &str) -> Option<&'a Expr> {
    let key = Symbol::new(name.to_owned());
    fields
        .iter()
        .find_map(|(candidate, value)| match candidate {
            Expr::Symbol(symbol) if symbol == &key => Some(value),
            _ => None,
        })
}

fn transport_kind(fields: &[(Expr, Expr)]) -> Result<String> {
    match required_field(fields, "kind")? {
        Expr::String(value) => Ok(value.clone()),
        Expr::Symbol(symbol) if symbol.namespace.is_none() => Ok(symbol.name.to_string()),
        Expr::Symbol(symbol) => Ok(symbol.to_string()),
        _ => Err(Error::TypeMismatch {
            expected: "transport kind",
            found: "invalid transport kind",
        }),
    }
}

fn parse_symbol_text(text: &str) -> Symbol {
    match text.split_once('/') {
        Some((namespace, name)) if !namespace.is_empty() && !name.is_empty() => {
            Symbol::qualified(namespace.to_owned(), name.to_owned())
        }
        _ => Symbol::new(text.to_owned()),
    }
}
