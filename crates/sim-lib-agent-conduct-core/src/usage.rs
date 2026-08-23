use std::collections::BTreeMap;
use std::fmt;

use sim_citizen_derive::Citizen;
use sim_kernel::{Error, Expr, NumberLiteral, Result, Symbol};

/// A non-negative integer usage quantity with an explicit unit dimension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UsageQuantity {
    /// Open unit symbol.
    pub unit: Symbol,
    /// Exact non-negative quantity.
    pub amount: u64,
}

/// Errors produced by usage and budget validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UsageError {
    /// A dimension appeared more than once.
    Duplicate(Symbol),
    /// A charge was not admitted by the effective limit.
    Exceeded {
        /// Dimension that exceeded its limit.
        unit: Symbol,
        /// Quantity already consumed.
        used: u64,
        /// Proposed additional charge.
        charge: u64,
        /// Effective pointwise limit.
        limit: u64,
    },
    /// An encoded usage expression was malformed.
    Malformed(String),
}

impl fmt::Display for UsageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate(unit) => write!(f, "duplicate usage dimension {unit}"),
            Self::Exceeded {
                unit,
                used,
                charge,
                limit,
            } => write!(f, "usage {unit} would exceed {limit}: {used} + {charge}"),
            Self::Malformed(message) => f.write_str(message),
        }
    }
}
impl std::error::Error for UsageError {}

/// Pointwise domain budget. Missing dimensions are unbounded at this layer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Citizen)]
#[citizen(symbol = "agent/UsageBudget", version = 1)]
pub struct AgentUsageBudget {
    #[citizen(with = "quantity_field")]
    limits: Vec<UsageQuantity>,
}

impl AgentUsageBudget {
    /// Constructs a checked budget.
    pub fn new(limits: Vec<UsageQuantity>) -> std::result::Result<Self, UsageError> {
        unique(&limits)?;
        Ok(Self { limits })
    }
    /// Returns the limits in canonical unit order.
    pub fn limits(&self) -> &[UsageQuantity] {
        &self.limits
    }
    /// Narrows two layers pointwise; absence remains unbounded only where both are absent.
    pub fn narrow(&self, later: &Self) -> Self {
        let mut values = map(&self.limits);
        for (unit, amount) in map(&later.limits) {
            values
                .entry(unit)
                .and_modify(|old| *old = (*old).min(amount))
                .or_insert(amount);
        }
        Self {
            limits: values
                .into_iter()
                .map(|(unit, amount)| UsageQuantity { unit, amount })
                .collect(),
        }
    }
    /// Checks whether current usage plus a charge fits this layer.
    pub fn admit(
        &self,
        usage: &AgentUsage,
        charge: &UsageQuantity,
    ) -> std::result::Result<(), UsageError> {
        let Some(limit) = self.limits.iter().find(|value| value.unit == charge.unit) else {
            return Ok(());
        };
        let used = usage.amount(&charge.unit);
        if used
            .checked_add(charge.amount)
            .is_none_or(|next| next > limit.amount)
        {
            return Err(UsageError::Exceeded {
                unit: charge.unit.clone(),
                used,
                charge: charge.amount,
                limit: limit.amount,
            });
        }
        Ok(())
    }
}

/// Exact accumulated domain usage.
#[derive(Clone, Debug, Default, PartialEq, Eq, Citizen)]
#[citizen(symbol = "agent/Usage", version = 1)]
pub struct AgentUsage {
    #[citizen(with = "quantity_field")]
    quantities: Vec<UsageQuantity>,
}

impl AgentUsage {
    /// Constructs checked usage.
    pub fn new(quantities: Vec<UsageQuantity>) -> std::result::Result<Self, UsageError> {
        unique(&quantities)?;
        Ok(Self { quantities })
    }
    /// Returns a dimension's quantity, or zero when absent.
    pub fn amount(&self, unit: &Symbol) -> u64 {
        self.quantities
            .iter()
            .find(|q| &q.unit == unit)
            .map_or(0, |q| q.amount)
    }
    /// Applies an admitted charge.
    pub fn charge(
        &mut self,
        budget: &AgentUsageBudget,
        charge: UsageQuantity,
    ) -> std::result::Result<(), UsageError> {
        budget.admit(self, &charge)?;
        if let Some(value) = self
            .quantities
            .iter_mut()
            .find(|value| value.unit == charge.unit)
        {
            value.amount += charge.amount;
        } else {
            self.quantities.push(charge);
            self.quantities.sort_by_key(|a| a.unit.to_string());
        }
        Ok(())
    }
    /// Returns exact quantities.
    pub fn quantities(&self) -> &[UsageQuantity] {
        &self.quantities
    }
}

fn unique(values: &[UsageQuantity]) -> std::result::Result<(), UsageError> {
    let mut seen = BTreeMap::new();
    for value in values {
        if seen.insert(value.unit.to_string(), ()).is_some() {
            return Err(UsageError::Duplicate(value.unit.clone()));
        }
    }
    Ok(())
}
fn map(values: &[UsageQuantity]) -> BTreeMap<Symbol, u64> {
    values.iter().map(|q| (q.unit.clone(), q.amount)).collect()
}

pub(crate) mod quantity_field {
    use super::*;
    pub fn encode(values: &[UsageQuantity]) -> Expr {
        Expr::List(
            values
                .iter()
                .map(|q| Expr::List(vec![Expr::Symbol(q.unit.clone()), integer(q.amount)]))
                .collect(),
        )
    }
    pub fn decode(expr: &Expr) -> Result<Vec<UsageQuantity>> {
        let Expr::List(rows) = expr else {
            return Err(Error::Eval("usage quantities must be a list".into()));
        };
        let mut values = Vec::new();
        for row in rows {
            let Expr::List(parts) = row else {
                return Err(Error::Eval("usage quantity must be (unit amount)".into()));
            };
            if let [Expr::Symbol(unit), Expr::Number(number)] = parts.as_slice() {
                let amount = number.canonical.parse::<u64>().map_err(|_| {
                    Error::Eval("usage amount must be a non-negative u64 integer".into())
                })?;
                values.push(UsageQuantity {
                    unit: unit.clone(),
                    amount,
                });
            } else {
                return Err(Error::Eval(
                    "usage quantity must contain a symbol and integer".into(),
                ));
            }
        }
        unique(&values).map_err(|err| Error::Eval(err.to_string()))?;
        values.sort_by_key(|a| a.unit.to_string());
        Ok(values)
    }
    fn integer(value: u64) -> Expr {
        Expr::Number(NumberLiteral {
            domain: Symbol::qualified("citizen", "int"),
            canonical: value.to_string(),
        })
    }
}
