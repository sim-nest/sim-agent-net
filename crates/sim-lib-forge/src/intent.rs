use sim_citizen::CitizenField;
use sim_citizen_derive::Citizen;
use sim_kernel::{ContentId, Error, Expr, Result, Symbol};

/// Verification and approval state for a compiled intent artifact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IntentStatus {
    /// The packet is lifted and structurally accepted, but not semantically
    /// verified.
    #[default]
    Candidate,
    /// The packet has passed the semantic probes attached to the artifact.
    Verified,
    /// A human has approved the verified artifact for compile-once reuse.
    Golden,
}

impl CitizenField for IntentStatus {
    fn encode_field(&self) -> Expr {
        Expr::Symbol(self.as_symbol())
    }

    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        let Expr::Symbol(symbol) = expr else {
            return Err(sim_citizen::field_error(
                field,
                "expected intent status symbol",
            ));
        };
        Self::from_symbol(symbol)
            .ok_or_else(|| sim_citizen::field_error(field, format!("unknown status {symbol}")))
    }
}

impl IntentStatus {
    /// Returns the stable symbol used in citizen field encodings.
    pub fn as_symbol(self) -> Symbol {
        match self {
            Self::Candidate => Symbol::qualified("forge", "candidate"),
            Self::Verified => Symbol::qualified("forge", "verified"),
            Self::Golden => Symbol::qualified("forge", "golden"),
        }
    }

    /// Parses a stable status symbol.
    pub fn from_symbol(symbol: &Symbol) -> Option<Self> {
        if *symbol == Symbol::qualified("forge", "candidate") {
            Some(Self::Candidate)
        } else if *symbol == Symbol::qualified("forge", "verified") {
            Some(Self::Verified)
        } else if *symbol == Symbol::qualified("forge", "golden") {
            Some(Self::Golden)
        } else {
            None
        }
    }
}

/// Named, versioned wrapper around a structurally checked BRIDGE packet.
#[derive(Clone, Debug, PartialEq, Citizen)]
#[citizen(symbol = "forge/CompiledIntent", version = 1)]
pub struct CompiledIntent {
    /// Stable symbolic name for the intent.
    pub name: Symbol,
    /// Monotonic artifact version for this intent name.
    pub version: u32,
    /// Content id of the normalized prose the packet was lifted from.
    #[citizen(with = "content_id_field")]
    pub source: ContentId,
    /// Content id of the BRIDGE packet used as the compiled IR.
    #[citizen(with = "content_id_field")]
    pub packet: ContentId,
    /// Stable identifiers for the Check, Evidence, or Vote parts that must pass.
    pub verifiers: Vec<Symbol>,
    /// Content ids of probe cases used for semantic verification.
    #[citizen(with = "content_id_vec_field")]
    pub probes: Vec<ContentId>,
    /// Promotion state for the artifact.
    pub status: IntentStatus,
    /// Optional model card content id for the compiler used during lifting.
    #[citizen(with = "optional_content_id_field")]
    pub compiler_card: Option<ContentId>,
    /// Optional human approval record required for golden reuse.
    #[citizen(with = "optional_content_id_field")]
    pub approval: Option<ContentId>,
}

impl Default for CompiledIntent {
    fn default() -> Self {
        Self {
            name: Symbol::qualified("forge", "fixture"),
            version: 1,
            source: fixture_content_id(1),
            packet: fixture_content_id(2),
            verifiers: Vec::new(),
            probes: Vec::new(),
            status: IntentStatus::Candidate,
            compiler_card: None,
            approval: None,
        }
    }
}

fn fixture_content_id(byte: u8) -> ContentId {
    ContentId::from_bytes(Symbol::qualified("core", "sha256"), [byte; 32])
}

mod content_id_field {
    use super::*;
    use sim_value::build::entry as field;

    pub(crate) fn encode(id: &ContentId) -> Expr {
        Expr::Map(vec![
            field("algorithm", Expr::Symbol(id.algorithm.clone())),
            field("bytes", Expr::Bytes(id.bytes.to_vec())),
        ])
    }

    pub(crate) fn decode(expr: &Expr) -> Result<ContentId> {
        let Expr::Map(entries) = expr else {
            return Err(Error::Eval("content id field must be a map".to_owned()));
        };
        let algorithm = required_symbol(entries, "algorithm")?;
        let bytes = required_bytes(entries, "bytes")?;
        Ok(ContentId::from_bytes(algorithm, bytes))
    }

    fn required_symbol(entries: &[(Expr, Expr)], name: &'static str) -> Result<Symbol> {
        match required_entry(entries, name)? {
            Expr::Symbol(symbol) => Ok(symbol.clone()),
            _ => Err(sim_citizen::field_error(name, "expected symbol")),
        }
    }

    fn required_bytes(entries: &[(Expr, Expr)], name: &'static str) -> Result<[u8; 32]> {
        match required_entry(entries, name)? {
            Expr::Bytes(bytes) if bytes.len() == 32 => {
                let mut digest = [0_u8; 32];
                digest.copy_from_slice(bytes);
                Ok(digest)
            }
            Expr::Bytes(bytes) => Err(sim_citizen::field_error(
                name,
                format!("expected 32 digest bytes, found {}", bytes.len()),
            )),
            _ => Err(sim_citizen::field_error(name, "expected bytes")),
        }
    }

    fn required_entry<'a>(entries: &'a [(Expr, Expr)], name: &'static str) -> Result<&'a Expr> {
        let key = Expr::Symbol(Symbol::new(name.to_owned()));
        entries
            .iter()
            .find_map(|(entry_key, value)| (entry_key == &key).then_some(value))
            .ok_or_else(|| sim_citizen::field_error(name, "missing content id field"))
    }
}

mod content_id_vec_field {
    use super::*;

    pub(crate) fn encode(ids: &[ContentId]) -> Expr {
        Expr::List(ids.iter().map(content_id_field::encode).collect())
    }

    pub(crate) fn decode(expr: &Expr) -> Result<Vec<ContentId>> {
        let Expr::List(items) = expr else {
            return Err(Error::Eval(
                "content id list field must be a list".to_owned(),
            ));
        };
        items.iter().map(content_id_field::decode).collect()
    }
}

mod optional_content_id_field {
    use super::*;

    pub(crate) fn encode(id: &Option<ContentId>) -> Expr {
        id.as_ref()
            .map(content_id_field::encode)
            .unwrap_or(Expr::Nil)
    }

    pub(crate) fn decode(expr: &Expr) -> Result<Option<ContentId>> {
        match expr {
            Expr::Nil => Ok(None),
            other => content_id_field::decode(other).map(Some),
        }
    }
}
