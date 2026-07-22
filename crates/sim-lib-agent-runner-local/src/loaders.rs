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
                Export::Class { symbol, .. } => ("class".to_owned(), symbol),
                Export::Function { symbol, .. } => ("function".to_owned(), symbol),
                Export::Macro { symbol, .. } => ("macro".to_owned(), symbol),
                Export::Shape { symbol, .. } => ("shape".to_owned(), symbol),
                Export::Codec { symbol, .. } => ("codec".to_owned(), symbol),
                Export::NumberDomain { symbol, .. } => ("number-domain".to_owned(), symbol),
                Export::Value { symbol } => ("value".to_owned(), symbol),
                Export::Site { symbol, .. } => ("site".to_owned(), symbol),
                Export::Open { kind, symbol } => (kind.symbol().as_qualified_str(), symbol),
            };
            Expr::Map(vec![
                symbol_entry("kind", Expr::String(kind)),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::{AbiVersion, ExportKind, Version};

    #[test]
    fn manifest_encoder_preserves_open_export_kind() {
        let manifest = LibManifest {
            id: Symbol::new("example"),
            version: Version("0.1.0".to_owned()),
            abi: AbiVersion { major: 1, minor: 0 },
            target: LibTarget::Native,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: vec![Export::Open {
                kind: ExportKind::new(Symbol::qualified("loader", "asset")),
                symbol: Symbol::qualified("example", "asset"),
            }],
        };

        let Expr::Map(entries) = manifest_to_expr(&manifest) else {
            panic!("manifest should encode as a map");
        };
        let exports = entries
            .iter()
            .find_map(|(key, value)| match (key, value) {
                (Expr::Symbol(symbol), Expr::List(exports))
                    if symbol.name.as_ref() == "exports" =>
                {
                    Some(exports)
                }
                _ => None,
            })
            .expect("exports entry");
        let [Expr::Map(export)] = exports.as_slice() else {
            panic!("manifest should encode one export map");
        };

        assert!(export.contains(&(
            Expr::Symbol(Symbol::new("kind")),
            Expr::String("loader/asset".to_owned()),
        )));
        assert!(export.contains(&(
            Expr::Symbol(Symbol::new("symbol")),
            Expr::Symbol(Symbol::qualified("example", "asset")),
        )));
    }
}
