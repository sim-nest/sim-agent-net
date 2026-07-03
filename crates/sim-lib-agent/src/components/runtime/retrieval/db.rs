use super::query::decode_query;
use crate::{FILE_READ_CAPABILITY, memory::io_error};
use sim_codec_binary::decode_frame;
use sim_kernel::{CapabilityName, Cx, Error, Expr, Result, Symbol};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) fn file_result_expr(cx: &Cx, root: Option<&PathBuf>, expr: Expr) -> Result<Expr> {
    cx.require(&CapabilityName::new(FILE_READ_CAPABILITY))?;
    let path = match expr {
        Expr::String(text) => PathBuf::from(text),
        Expr::Symbol(symbol) => PathBuf::from(symbol.to_string()),
        _ => {
            return Err(Error::Eval(
                "retriever/file expects a string or symbol path".to_owned(),
            ));
        }
    };
    let full_path = if path.is_absolute() {
        path
    } else if let Some(root) = root {
        root.join(path)
    } else {
        path
    };
    let text = fs::read_to_string(&full_path).map_err(io_error)?;
    Ok(Expr::Map(vec![
        (
            Expr::Symbol(Symbol::new("path")),
            Expr::String(full_path.display().to_string()),
        ),
        (Expr::Symbol(Symbol::new("text")), Expr::String(text)),
    ]))
}

pub(super) fn db_result_expr(cx: &Cx, path: &Path, expr: Expr) -> Result<Expr> {
    cx.require(&CapabilityName::new(FILE_READ_CAPABILITY))?;
    let (query_expr, limit) = decode_query(expr)?;
    let query = tokenize(&query_text(&query_expr));
    let records = load_db_records(path)?;
    let mut matches = Vec::new();
    for (id, text) in records {
        let haystack = text.to_ascii_lowercase();
        if query.iter().all(|term| haystack.contains(term)) {
            matches.push(Expr::Map(vec![
                (Expr::Symbol(Symbol::new("id")), Expr::String(id)),
                (Expr::Symbol(Symbol::new("text")), Expr::String(text)),
            ]));
        }
    }
    Ok(Expr::List(
        matches
            .into_iter()
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .collect(),
    ))
}

fn query_text(expr: &Expr) -> String {
    match expr {
        Expr::String(text) => text.clone(),
        Expr::Symbol(symbol) => symbol.to_string(),
        other => format!("{other:?}"),
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(|term| term.to_ascii_lowercase())
        .collect()
}

fn load_db_records(path: &Path) -> Result<Vec<(String, String)>> {
    let bytes = fs::read(path).map_err(io_error)?;
    if bytes.iter().copied().any(|byte| byte == b'\n') {
        return load_jsonl_records(&bytes);
    }
    load_binary_records(&bytes)
}

fn load_jsonl_records(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let mut records = Vec::new();
    for line in String::from_utf8_lossy(bytes)
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let value: serde_json::Value =
            serde_json::from_str(line).map_err(|error| Error::HostError(error.to_string()))?;
        let id = value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| Error::Eval("db record missing string id".to_owned()))?;
        let text = value
            .get("text")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| Error::Eval("db record missing string text".to_owned()))?;
        records.push((id.to_owned(), text.to_owned()));
    }
    Ok(records)
}

fn load_binary_records(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let mut offset = 0usize;
    let mut records = Vec::new();
    while offset < bytes.len() {
        if bytes.len().saturating_sub(offset) < 8 {
            return Err(Error::HostError(
                "db binary record ended with a partial length header".to_owned(),
            ));
        }
        let len = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        offset += 8;
        let len = usize::try_from(len)
            .map_err(|_| Error::HostError("db binary record length exceeds usize".to_owned()))?;
        if bytes.len().saturating_sub(offset) < len {
            return Err(Error::HostError(
                "db binary record ended with a partial record".to_owned(),
            ));
        }
        let (_, expr) = decode_frame(sim_kernel::CodecId(0), &bytes[offset..offset + len])?;
        offset += len;
        let Expr::Map(entries) = expr else {
            return Err(Error::Eval("db binary records must be maps".to_owned()));
        };
        let mut id = None;
        let mut text = None;
        for (key, value) in entries {
            match (key, value) {
                (Expr::Symbol(symbol), Expr::String(value)) if symbol.name.as_ref() == "id" => {
                    id = Some(value);
                }
                (Expr::Symbol(symbol), Expr::String(value)) if symbol.name.as_ref() == "text" => {
                    text = Some(value);
                }
                _ => {}
            }
        }
        records.push((
            id.ok_or_else(|| Error::Eval("db binary record missing id".to_owned()))?,
            text.ok_or_else(|| Error::Eval("db binary record missing text".to_owned()))?,
        ));
    }
    Ok(records)
}
