use sim_kernel::{Cx, Error, Result, Symbol};

pub(crate) fn installed_server_codecs(cx: &Cx) -> Vec<Symbol> {
    cx.registry().codecs().keys().cloned().collect()
}

pub(crate) fn default_server_codec(codecs: &[Symbol]) -> Result<Symbol> {
    codecs
        .iter()
        .find(|symbol| **symbol == Symbol::qualified("codec", "lisp"))
        .cloned()
        .or_else(|| codecs.first().cloned())
        .ok_or_else(|| Error::Eval("server/start: no installed codecs available".to_owned()))
}

pub(crate) fn ensure_installed_codec(cx: &Cx, codec: &Symbol) -> Result<()> {
    if cx.registry().codec_by_symbol(codec).is_some() {
        Ok(())
    } else {
        Err(Error::Eval(format!("server/start: unknown codec {codec}")))
    }
}

pub(crate) fn normalize_codec_expr(cx: &Cx, symbol: &Symbol) -> Option<Symbol> {
    if cx.registry().codec_by_symbol(symbol).is_some() {
        return Some(symbol.clone());
    }
    let qualified = Symbol::qualified("codec", symbol.name.to_string());
    cx.registry().codec_by_symbol(&qualified).map(|_| qualified)
}

pub(crate) fn default_connection_codec(codecs: &[Symbol]) -> Result<Symbol> {
    [
        Symbol::qualified("codec", "json"),
        Symbol::qualified("codec", "binary"),
        Symbol::qualified("codec", "lisp"),
    ]
    .into_iter()
    .find(|symbol| codecs.iter().any(|codec| codec == symbol))
    .or_else(|| codecs.first().cloned())
    .ok_or_else(|| Error::Eval("server/connect: no installed codecs available".to_owned()))
}

pub(crate) fn negotiation_offer_codecs(codecs: &[Symbol]) -> Vec<Symbol> {
    let preferred = [
        Symbol::qualified("codec", "binary"),
        Symbol::qualified("codec", "lisp"),
        Symbol::qualified("codec", "json"),
    ];
    let mut ordered = Vec::new();
    for symbol in preferred {
        if codecs.iter().any(|codec| codec == &symbol) {
            ordered.push(symbol);
        }
    }
    for codec in codecs {
        if !ordered.iter().any(|existing| existing == codec) {
            ordered.push(codec.clone());
        }
    }
    ordered
}

pub(crate) fn choose_codec(
    default_codec: Symbol,
    supported_codecs: &[Symbol],
    explicit: Option<Symbol>,
    preferred: &[Symbol],
) -> Result<Symbol> {
    if let Some(codec) = explicit {
        if supported_codecs.iter().any(|candidate| candidate == &codec) {
            return Ok(codec);
        }
        return Err(Error::Eval(format!(
            "connection target does not support codec {codec}"
        )));
    }
    if let Some(codec) = preferred
        .iter()
        .find(|codec| supported_codecs.iter().any(|candidate| candidate == *codec))
    {
        return Ok(codec.clone());
    }
    Ok(default_codec)
}
