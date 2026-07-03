//! Native-ABI loader helpers required by the generated export shim.

use crate::kernel::{
    CapabilityName, Dependency, Export, Expr, LibManifest, LibTarget, NativeAbiCallResponse,
    NumberLiteral, Result, Symbol, native_abi_owned_bytes,
};

/// Encodes a library manifest as the response payload expected by the native ABI.
pub fn encode_native_manifest_response(manifest: &LibManifest) -> Result<NativeAbiCallResponse> {
    let bytes = crate::codec_binary::encode_frame(&manifest_to_expr(manifest))?.0;
    Ok(NativeAbiCallResponse::success(native_abi_owned_bytes(
        bytes,
    )))
}

fn manifest_to_expr(manifest: &LibManifest) -> Expr {
    Expr::Map(vec![
        symbol_entry("id", Expr::Symbol(manifest.id.clone())),
        symbol_entry("version", Expr::String(manifest.version.0.clone())),
        symbol_entry("abi-major", number_expr(manifest.abi.major)),
        symbol_entry("abi-minor", number_expr(manifest.abi.minor)),
        symbol_entry("target", Expr::String(lib_target_name(&manifest.target))),
        symbol_entry("requires", Expr::List(requires_to_expr(&manifest.requires))),
        symbol_entry(
            "capabilities",
            Expr::List(capabilities_to_expr(&manifest.capabilities)),
        ),
        symbol_entry("exports", Expr::List(exports_to_expr(&manifest.exports))),
    ])
}

fn requires_to_expr(requires: &[Dependency]) -> Vec<Expr> {
    requires
        .iter()
        .map(|dependency| {
            Expr::Map(vec![
                symbol_entry("id", Expr::Symbol(dependency.id.clone())),
                symbol_entry(
                    "minimum-version",
                    dependency
                        .minimum_version
                        .as_ref()
                        .map(|version| Expr::String(version.0.clone()))
                        .unwrap_or(Expr::Nil),
                ),
            ])
        })
        .collect()
}

fn capabilities_to_expr(capabilities: &[CapabilityName]) -> Vec<Expr> {
    capabilities
        .iter()
        .map(|capability| Expr::String(capability.as_str().to_owned()))
        .collect()
}

fn exports_to_expr(exports: &[Export]) -> Vec<Expr> {
    exports
        .iter()
        .map(|export| {
            let (kind, symbol) = match export {
                Export::Class { symbol, .. } => ("class", symbol),
                Export::Function { symbol, .. } => ("function", symbol),
                Export::Macro { symbol, .. } => ("macro", symbol),
                Export::Shape { symbol, .. } => ("shape", symbol),
                Export::Codec { symbol, .. } => ("codec", symbol),
                Export::NumberDomain { symbol, .. } => ("number-domain", symbol),
                Export::Value { symbol } => ("value", symbol),
                Export::Site { symbol, .. } => ("site", symbol),
            };
            Expr::Map(vec![
                symbol_entry("kind", Expr::String(kind.to_owned())),
                symbol_entry("symbol", Expr::Symbol(symbol.clone())),
            ])
        })
        .collect()
}

fn symbol_entry(key: &str, value: Expr) -> (Expr, Expr) {
    (Expr::Symbol(Symbol::new(key)), value)
}

fn number_expr(value: impl ToString) -> Expr {
    Expr::Number(NumberLiteral {
        domain: Symbol::qualified("numbers", "f64"),
        canonical: value.to_string(),
    })
}

fn lib_target_name(target: &LibTarget) -> String {
    target.to_symbol().as_qualified_str()
}
