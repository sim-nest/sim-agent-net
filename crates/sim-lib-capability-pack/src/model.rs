use std::{collections::BTreeSet, fmt};

use sim_citizen::CitizenField;
use sim_citizen_derive::Citizen;
use sim_kernel::{Error, Expr, Result, Symbol};

/// Current capability-pack schema version.
pub const CURRENT_PACK_VERSION: u64 = 1;

/// Immutable content identity. Only canonical `sha256:<64 lowercase hex>` ids are admitted.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ContentId(String);

impl ContentId {
    /// Parses a canonical immutable content id.
    pub fn parse(value: impl Into<String>) -> std::result::Result<Self, String> {
        let value = value.into();
        let digest = value
            .strip_prefix("sha256:")
            .ok_or_else(|| "mutable locator: expected sha256 content id".to_owned())?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("content id must contain 64 lowercase hexadecimal digits".to_owned());
        }
        Ok(Self(value))
    }

    /// Returns the canonical id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An import edge with a human-facing alias and an authority ceiling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    /// Local alias.
    pub alias: Symbol,
    /// Immutable target identity.
    pub content: ContentId,
    /// Capabilities this import is permitted to retain.
    pub ceiling: BTreeSet<Symbol>,
}

/// A declared runtime library and its resolved entry Shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LibrarySpec {
    /// Stable library id.
    pub id: Symbol,
    /// Index route to the loadable implementation.
    pub route: Symbol,
    /// Input/constructor Shape id.
    pub shape: Symbol,
    /// Effects the library may perform.
    pub effects: BTreeSet<Symbol>,
}

/// A named output supplied by one library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackOutput {
    /// Output name.
    pub name: Symbol,
    /// Producing library.
    pub library: Symbol,
}

/// Surface disclosure attached to the pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackSurface {
    /// Surface id.
    pub id: Symbol,
    /// Declared disclosure class.
    pub disclosure: Symbol,
}

/// A capability-flow claim checked before loading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackClaim {
    /// Claim id.
    pub id: Symbol,
    /// Required capability.
    pub capability: Symbol,
    /// Library making the claim.
    pub library: Symbol,
}

/// Checked success or refusal specimen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckSpecimen {
    /// Specimen id.
    pub id: Symbol,
    /// Expected outcome (`success` or `refusal`).
    pub outcome: Symbol,
}

/// Human-operated fallback when automated routes are unavailable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManualFallback {
    /// Trigger or route gap.
    pub when: Symbol,
    /// Non-empty operator instruction.
    pub instruction: String,
}

/// Versioned, content-addressed capability composition.
#[derive(Clone, Debug, PartialEq, Eq, Citizen)]
#[citizen(symbol = "capability-pack/Pack", version = 1)]
pub struct CapabilityPack {
    /// Schema version.
    pub version: u64,
    /// Claimed content identity, verified by the injected directory.
    pub content: String,
    /// Imports.
    pub imports: Vec<Expr>,
    /// Runtime library declarations.
    pub libraries: Vec<Expr>,
    /// Capability claims.
    pub claims: Vec<Expr>,
    /// Output declarations.
    pub outputs: Vec<Expr>,
    /// Surface declarations.
    pub surfaces: Vec<Expr>,
    /// Checked specimens.
    pub specimens: Vec<Expr>,
    /// Manual fallbacks.
    pub fallbacks: Vec<Expr>,
}

impl Default for CapabilityPack {
    fn default() -> Self {
        Self {
            version: CURRENT_PACK_VERSION,
            content: format!("sha256:{}", "0".repeat(64)),
            imports: vec![],
            libraries: vec![],
            claims: vec![],
            outputs: vec![],
            surfaces: vec![],
            specimens: vec![],
            fallbacks: vec![],
        }
    }
}

impl CapabilityPack {
    /// Parses the typed import records carried as open kernel data.
    pub fn typed_imports(&self) -> std::result::Result<Vec<Import>, String> {
        self.imports.iter().map(parse_import).collect()
    }
    /// Parses library records.
    pub fn typed_libraries(&self) -> std::result::Result<Vec<LibrarySpec>, String> {
        self.libraries.iter().map(parse_library).collect()
    }
    /// Parses claim records.
    pub fn typed_claims(&self) -> std::result::Result<Vec<PackClaim>, String> {
        self.claims.iter().map(parse_claim).collect()
    }
    /// Parses output records.
    pub fn typed_outputs(&self) -> std::result::Result<Vec<PackOutput>, String> {
        self.outputs.iter().map(parse_output).collect()
    }
    /// Parses surface records.
    pub fn typed_surfaces(&self) -> std::result::Result<Vec<PackSurface>, String> {
        self.surfaces.iter().map(parse_surface).collect()
    }
    /// Parses specimen records.
    pub fn typed_specimens(&self) -> std::result::Result<Vec<CheckSpecimen>, String> {
        self.specimens.iter().map(parse_specimen).collect()
    }
    /// Parses fallback records.
    pub fn typed_fallbacks(&self) -> std::result::Result<Vec<ManualFallback>, String> {
        self.fallbacks.iter().map(parse_fallback).collect()
    }
}

impl CitizenField for CapabilityPack {
    fn encode_field(&self) -> Expr {
        Expr::List(vec![
            self.version.encode_field(),
            self.content.encode_field(),
            self.imports.encode_field(),
            self.libraries.encode_field(),
            self.claims.encode_field(),
            self.outputs.encode_field(),
            self.surfaces.encode_field(),
            self.specimens.encode_field(),
            self.fallbacks.encode_field(),
        ])
    }

    fn decode_field_expr(expr: &Expr, _field: &'static str) -> Result<Self> {
        let Expr::List(v) = expr else {
            return Err(Error::Eval("capability pack must be a list".to_owned()));
        };
        let [
            version,
            content,
            imports,
            libraries,
            claims,
            outputs,
            surfaces,
            specimens,
            fallbacks,
        ] = v.as_slice()
        else {
            return Err(Error::Eval("capability pack has wrong arity".to_owned()));
        };
        Ok(Self {
            version: u64::decode_field_expr(version, "version")?,
            content: String::decode_field_expr(content, "content")?,
            imports: Vec::<Expr>::decode_field_expr(imports, "imports")?,
            libraries: Vec::<Expr>::decode_field_expr(libraries, "libraries")?,
            claims: Vec::<Expr>::decode_field_expr(claims, "claims")?,
            outputs: Vec::<Expr>::decode_field_expr(outputs, "outputs")?,
            surfaces: Vec::<Expr>::decode_field_expr(surfaces, "surfaces")?,
            specimens: Vec::<Expr>::decode_field_expr(specimens, "specimens")?,
            fallbacks: Vec::<Expr>::decode_field_expr(fallbacks, "fallbacks")?,
        })
    }
}

fn list<'a>(expr: &'a Expr, n: usize, kind: &str) -> std::result::Result<&'a [Expr], String> {
    match expr {
        Expr::List(v) if v.len() == n => Ok(v),
        _ => Err(format!("{kind} must be a {n}-field list")),
    }
}
fn sym(e: &Expr, f: &'static str) -> std::result::Result<Symbol, String> {
    Symbol::decode_field_expr(e, f).map_err(|e| e.to_string())
}
fn text(e: &Expr, f: &'static str) -> std::result::Result<String, String> {
    String::decode_field_expr(e, f).map_err(|e| e.to_string())
}
fn syms(e: &Expr, f: &'static str) -> std::result::Result<BTreeSet<Symbol>, String> {
    Ok(Vec::<Symbol>::decode_field_expr(e, f)
        .map_err(|e| e.to_string())?
        .into_iter()
        .collect())
}
fn parse_import(e: &Expr) -> std::result::Result<Import, String> {
    let v = list(e, 3, "import")?;
    Ok(Import {
        alias: sym(&v[0], "alias")?,
        content: ContentId::parse(text(&v[1], "content")?)?,
        ceiling: syms(&v[2], "ceiling")?,
    })
}
fn parse_library(e: &Expr) -> std::result::Result<LibrarySpec, String> {
    let v = list(e, 4, "library")?;
    Ok(LibrarySpec {
        id: sym(&v[0], "id")?,
        route: sym(&v[1], "route")?,
        shape: sym(&v[2], "shape")?,
        effects: syms(&v[3], "effects")?,
    })
}
fn parse_claim(e: &Expr) -> std::result::Result<PackClaim, String> {
    let v = list(e, 3, "claim")?;
    Ok(PackClaim {
        id: sym(&v[0], "id")?,
        capability: sym(&v[1], "capability")?,
        library: sym(&v[2], "library")?,
    })
}
fn parse_output(e: &Expr) -> std::result::Result<PackOutput, String> {
    let v = list(e, 2, "output")?;
    Ok(PackOutput {
        name: sym(&v[0], "name")?,
        library: sym(&v[1], "library")?,
    })
}
fn parse_surface(e: &Expr) -> std::result::Result<PackSurface, String> {
    let v = list(e, 2, "surface")?;
    Ok(PackSurface {
        id: sym(&v[0], "surface")?,
        disclosure: sym(&v[1], "disclosure")?,
    })
}
fn parse_specimen(e: &Expr) -> std::result::Result<CheckSpecimen, String> {
    let v = list(e, 2, "specimen")?;
    Ok(CheckSpecimen {
        id: sym(&v[0], "specimen")?,
        outcome: sym(&v[1], "outcome")?,
    })
}
fn parse_fallback(e: &Expr) -> std::result::Result<ManualFallback, String> {
    let v = list(e, 2, "fallback")?;
    Ok(ManualFallback {
        when: sym(&v[0], "when")?,
        instruction: text(&v[1], "instruction")?,
    })
}

impl CitizenField for ContentId {
    fn encode_field(&self) -> Expr {
        self.0.encode_field()
    }
    fn decode_field_expr(expr: &Expr, field: &'static str) -> Result<Self> {
        Self::parse(String::decode_field_expr(expr, field)?).map_err(Error::Eval)
    }
}
