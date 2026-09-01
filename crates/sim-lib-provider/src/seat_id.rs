use sim_citizen::CitizenField;
use sim_citizen_derive::Citizen;
use sim_kernel::{Error, Expr, Result, Symbol};
use std::fmt;
use std::str::FromStr;

/// Stable identity of one independently selectable provider seat.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "provider/ProviderSeatId", version = 1)]
pub struct ProviderSeatId {
    /// Open provider family symbol, conventionally `provider/<family-name>`.
    pub family: Symbol,
    /// Operator-chosen seat label within the family.
    pub label: String,
    /// Open extension fields preserved by provider-aware tooling.
    pub extra: Vec<(Expr, Expr)>,
}

impl Default for ProviderSeatId {
    fn default() -> Self {
        Self::new(Symbol::qualified("provider", "fixture"), "fixture")
            .expect("static provider seat fixture is valid")
    }
}

impl ProviderSeatId {
    /// Constructs a seat id after validating its printable label.
    pub fn new(family: Symbol, label: impl Into<String>) -> Result<Self> {
        let label = label.into();
        validate_label(&label)?;
        Ok(Self {
            family,
            label,
            extra: Vec::new(),
        })
    }
}

impl fmt::Display for ProviderSeatId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "seat:{}#{}", self.family.name, self.label)
    }
}

impl FromStr for ProviderSeatId {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        let body = value
            .strip_prefix("seat:")
            .ok_or_else(|| Error::Eval("provider seat id must start with seat:".to_owned()))?;
        let (family, label) = body
            .split_once('#')
            .ok_or_else(|| Error::Eval("provider seat id must contain #".to_owned()))?;
        if family.is_empty() || family.chars().any(char::is_whitespace) {
            return Err(Error::Eval(
                "provider seat family name must be non-empty and contain no whitespace".to_owned(),
            ));
        }
        Self::new(Symbol::qualified("provider", family), label)
    }
}

impl CitizenField for ProviderSeatId {
    fn encode_field(&self) -> Expr {
        Expr::List(vec![
            self.family.encode_field(),
            self.label.encode_field(),
            self.extra.encode_field(),
        ])
    }

    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        let Expr::List(items) = expr else {
            return Err(sim_citizen::field_error(
                field,
                "expected provider seat id list",
            ));
        };
        let [family, label, extra] = items.as_slice() else {
            return Err(sim_citizen::field_error(
                field,
                "expected 3 provider seat id fields",
            ));
        };
        let family = Symbol::decode_field_expr(family, field)?;
        let label = String::decode_field_expr(label, field)?;
        let mut id = Self::new(family, label)
            .map_err(|error| sim_citizen::field_error(field, error.to_string()))?;
        id.extra = Vec::<(Expr, Expr)>::decode_field_expr(extra, field)?;
        Ok(id)
    }
}

fn validate_label(label: &str) -> Result<()> {
    if label.is_empty() {
        return Err(Error::Eval(
            "provider seat label must not be empty".to_owned(),
        ));
    }
    if label.contains('#') {
        return Err(Error::Eval(
            "provider seat label must not contain #".to_owned(),
        ));
    }
    if label.chars().any(char::is_whitespace) {
        return Err(Error::Eval(
            "provider seat label must not contain whitespace".to_owned(),
        ));
    }
    if label.chars().any(char::is_control) {
        return Err(Error::Eval(
            "provider seat label must not contain control characters".to_owned(),
        ));
    }
    Ok(())
}
