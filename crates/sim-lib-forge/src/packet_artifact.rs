use sim_codec::{DecodeBudget, DecodeLimits};
use sim_codec_bridge::{
    BridgeHeader, BridgePacket, BridgePart, BridgeProvenance, BridgeWarrant,
    canonical_packet_datum, content_id_string, packet_content_id, stamp_packet_cid,
};
use sim_kernel::{CodecId, ContentId, Cx, Datum, DatumStore, Error, Expr, Result, Symbol};

/// Stores a BRIDGE packet artifact in the context datum store and returns its
/// canonical packet content id.
pub fn store_packet_artifact(cx: &mut Cx, packet: &BridgePacket) -> Result<ContentId> {
    let id = packet_content_id(packet)?;
    let stored = cx
        .datum_store_mut()
        .intern(canonical_packet_datum(packet))?;
    if stored == id {
        Ok(id)
    } else {
        Err(Error::Eval(format!(
            "stored packet id {} did not match canonical id {}",
            content_id_string(&stored),
            content_id_string(&id)
        )))
    }
}

pub(crate) fn resolve_packet(cx: &mut Cx, id: &ContentId) -> Result<BridgePacket> {
    let Some(datum) = cx.datum_store().get(id)?.cloned() else {
        return Err(Error::Eval(format!(
            "compiled packet artifact {} is not interned",
            content_id_string(id)
        )));
    };
    let packet = packet_from_canonical_datum(&datum)?;
    let actual = packet_content_id(&packet)?;
    if &actual == id {
        stamp_packet_cid(&packet)
    } else {
        Err(Error::Eval(format!(
            "compiled packet artifact {} decoded as {}",
            content_id_string(id),
            content_id_string(&actual)
        )))
    }
}

fn packet_from_canonical_datum(datum: &Datum) -> Result<BridgePacket> {
    let fields = node_fields(datum, "bridge", "Packet", "BRIDGE packet datum")?;
    Ok(BridgePacket {
        header: header_from_datum(required_field(fields, "header", "BRIDGE packet datum")?)?,
        body: vector_field(
            required_field(fields, "body", "BRIDGE packet datum")?,
            "body",
        )?
        .iter()
        .map(part_from_datum)
        .collect::<Result<Vec<_>>>()?,
        warrant: warrant_from_datum(required_field(fields, "warrant", "BRIDGE packet datum")?)?,
    })
}

fn header_from_datum(datum: &Datum) -> Result<BridgeHeader> {
    let fields = node_fields(datum, "bridge", "Header", "BRIDGE header datum")?;
    Ok(BridgeHeader {
        cid: string_or_nil(required_field(fields, "cid", "BRIDGE header datum")?, "cid")?,
        move_kind: symbol_field(fields, "move", "BRIDGE header datum")?,
        from: string_field(fields, "from", "BRIDGE header datum")?,
        to: string_vec_field(fields, "to", "BRIDGE header datum")?,
        role: symbol_field(fields, "role", "BRIDGE header datum")?,
        parents: string_vec_field(fields, "parents", "BRIDGE header datum")?,
        task: symbol_field(fields, "task", "BRIDGE header datum")?,
        output: symbol_field(fields, "output", "BRIDGE header datum")?,
        ceiling: symbol_vec_field(fields, "ceiling", "BRIDGE header datum")?,
        context: symbol_vec_field(fields, "context", "BRIDGE header datum")?,
        provenance: provenance_from_datum(required_field(
            fields,
            "provenance",
            "BRIDGE header datum",
        )?)?,
    })
}

fn provenance_from_datum(datum: &Datum) -> Result<BridgeProvenance> {
    let fields = node_fields(datum, "bridge", "Provenance", "BRIDGE provenance datum")?;
    Ok(BridgeProvenance {
        author: symbol_field(fields, "author", "BRIDGE provenance datum")?,
        card: string_or_nil(
            required_field(fields, "card", "BRIDGE provenance datum")?,
            "card",
        )?,
    })
}

fn part_from_datum(datum: &Datum) -> Result<BridgePart> {
    let fields = node_fields(datum, "bridge", "Part", "BRIDGE part datum")?;
    Ok(BridgePart {
        id: symbol_field(fields, "id", "BRIDGE part datum")?,
        kind: symbol_field(fields, "kind", "BRIDGE part datum")?,
        payload: expr_json_field(fields, "payload", "BRIDGE part datum")?,
    })
}

fn warrant_from_datum(datum: &Datum) -> Result<Option<BridgeWarrant>> {
    if matches!(datum, Datum::Nil) {
        return Ok(None);
    }
    let fields = node_fields(datum, "bridge", "Warrant", "BRIDGE warrant datum")?;
    Ok(Some(BridgeWarrant {
        moves: content_id_field(fields, "moves", "BRIDGE warrant datum")?,
        frames: content_id_field(fields, "frames", "BRIDGE warrant datum")?,
        parts: vector_field(
            required_field(fields, "parts", "BRIDGE warrant datum")?,
            "parts",
        )?
        .iter()
        .map(warrant_part_from_datum)
        .collect::<Result<Vec<_>>>()?,
    }))
}

fn warrant_part_from_datum(datum: &Datum) -> Result<(Symbol, ContentId)> {
    let fields = node_fields(datum, "bridge", "WarrantPart", "BRIDGE warrant part datum")?;
    Ok((
        symbol_field(fields, "kind", "BRIDGE warrant part datum")?,
        content_id_field(fields, "cid", "BRIDGE warrant part datum")?,
    ))
}

fn content_id_from_datum(datum: &Datum, context: &str) -> Result<ContentId> {
    let fields = node_fields(datum, "core", "ContentId", context)?;
    let algorithm = symbol_field(fields, "algorithm", context)?;
    let bytes = match required_field(fields, "bytes", context)? {
        Datum::Bytes(bytes) if bytes.len() == 32 => {
            let mut digest = [0u8; 32];
            digest.copy_from_slice(bytes);
            digest
        }
        Datum::Bytes(bytes) => {
            return Err(Error::Eval(format!(
                "{context} bytes must contain 32 digest bytes, found {}",
                bytes.len()
            )));
        }
        _ => return Err(Error::Eval(format!("{context} bytes must be bytes"))),
    };
    Ok(ContentId::from_bytes(algorithm, bytes))
}

fn node_fields<'a>(
    datum: &'a Datum,
    namespace: &str,
    name: &str,
    context: &str,
) -> Result<&'a [(Symbol, Datum)]> {
    match datum {
        Datum::Node { tag, fields }
            if tag.namespace.as_deref() == Some(namespace) && tag.name.as_ref() == name =>
        {
            Ok(fields)
        }
        _ => Err(Error::Eval(format!(
            "{context} must be {namespace}/{name} node"
        ))),
    }
}

fn required_field<'a>(
    fields: &'a [(Symbol, Datum)],
    name: &str,
    context: &str,
) -> Result<&'a Datum> {
    fields
        .iter()
        .find_map(|(field, value)| {
            (field.namespace.is_none() && field.name.as_ref() == name).then_some(value)
        })
        .ok_or_else(|| Error::Eval(format!("{context} is missing {name}")))
}

fn symbol_field(fields: &[(Symbol, Datum)], name: &str, context: &str) -> Result<Symbol> {
    match required_field(fields, name, context)? {
        Datum::Symbol(symbol) => Ok(symbol.clone()),
        _ => Err(Error::Eval(format!(
            "{context} field {name} must be a symbol"
        ))),
    }
}

fn string_field(fields: &[(Symbol, Datum)], name: &str, context: &str) -> Result<String> {
    match required_field(fields, name, context)? {
        Datum::String(value) => Ok(value.clone()),
        _ => Err(Error::Eval(format!(
            "{context} field {name} must be a string"
        ))),
    }
}

fn string_or_nil(datum: &Datum, name: &str) -> Result<Option<String>> {
    match datum {
        Datum::Nil => Ok(None),
        Datum::String(value) => Ok(Some(value.clone())),
        _ => Err(Error::Eval(format!("field {name} must be a string or nil"))),
    }
}

fn string_vec_field(fields: &[(Symbol, Datum)], name: &str, context: &str) -> Result<Vec<String>> {
    vector_field(required_field(fields, name, context)?, name)?
        .iter()
        .map(|datum| match datum {
            Datum::String(value) => Ok(value.clone()),
            _ => Err(Error::Eval(format!(
                "{context} field {name} entries must be strings"
            ))),
        })
        .collect()
}

fn symbol_vec_field(fields: &[(Symbol, Datum)], name: &str, context: &str) -> Result<Vec<Symbol>> {
    vector_field(required_field(fields, name, context)?, name)?
        .iter()
        .map(|datum| match datum {
            Datum::Symbol(value) => Ok(value.clone()),
            _ => Err(Error::Eval(format!(
                "{context} field {name} entries must be symbols"
            ))),
        })
        .collect()
}

fn content_id_field(fields: &[(Symbol, Datum)], name: &str, context: &str) -> Result<ContentId> {
    content_id_from_datum(required_field(fields, name, context)?, context)
}

fn expr_json_field(fields: &[(Symbol, Datum)], name: &str, context: &str) -> Result<Expr> {
    let datum = required_field(fields, name, context)?;
    let expr_fields = node_fields(datum, "bridge", "ExprJson", context)?;
    let Datum::String(json) = required_field(expr_fields, "json", context)? else {
        return Err(Error::Eval(format!(
            "{context} expression payload json must be a string"
        )));
    };
    let value = serde_json::from_str(json)
        .map_err(|err| Error::Eval(format!("{context} expression json is invalid: {err}")))?;
    let mut budget = DecodeBudget::new(DecodeLimits::default());
    sim_codec_json::json_to_expr(CodecId(1), &value, &mut budget, 0)
}

fn vector_field<'a>(datum: &'a Datum, name: &str) -> Result<&'a [Datum]> {
    match datum {
        Datum::Vector(items) => Ok(items),
        _ => Err(Error::Eval(format!("{name} must be a vector"))),
    }
}
