use sim_codec_binary::{decode_frame, encode_frame};
use sim_kernel::{Cx, Error, Expr, Result, Value};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

pub(crate) type SharedEntries = Arc<Mutex<Vec<Expr>>>;
type BlackboardRegistry = Mutex<HashMap<String, SharedEntries>>;

pub(crate) fn shared_blackboard_entries(board: &str) -> SharedEntries {
    static BLACKBOARDS: OnceLock<BlackboardRegistry> = OnceLock::new();
    let boards = BLACKBOARDS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut boards = boards
        .lock()
        .map_err(|_| Error::PoisonedLock("blackboard registry"))
        .unwrap();
    boards
        .entry(board.to_owned())
        .or_insert_with(|| Arc::new(Mutex::new(Vec::new())))
        .clone()
}

pub(crate) fn memory_entries_append(entries: &SharedEntries, expr: Expr) -> Result<()> {
    lock_entries(entries, "memory entries")?.push(expr);
    Ok(())
}

pub(crate) fn memory_entries_recent(
    cx: &mut Cx,
    entries: &SharedEntries,
    count: u32,
) -> Result<Vec<Value>> {
    let entries = lock_entries(entries, "memory entries")?;
    let count = usize::try_from(count).unwrap_or(usize::MAX);
    entries[entries.len().saturating_sub(count)..]
        .iter()
        .map(|expr| crate::expr_to_value(cx, expr))
        .collect()
}

pub(crate) fn memory_entries_search(
    cx: &mut Cx,
    entries: &SharedEntries,
    query: Expr,
    k: u32,
) -> Result<Vec<Value>> {
    let terms = query_terms(&query);
    let entries = lock_entries(entries, "memory entries")?;
    let filtered = entries
        .iter()
        .rev()
        .filter(|expr| memory_entry_matches(expr, &terms))
        .take(usize::try_from(k).unwrap_or(usize::MAX))
        .cloned()
        .collect::<Vec<_>>();
    filtered
        .into_iter()
        .rev()
        .map(|expr| crate::expr_to_value(cx, &expr))
        .collect()
}

pub(crate) fn memory_entries_snapshot(entries: &SharedEntries) -> Result<Expr> {
    Ok(Expr::List(lock_entries(entries, "memory entries")?.clone()))
}

pub(crate) fn memory_entries_restore(entries: &SharedEntries, snap: Expr) -> Result<()> {
    replace_memory_entries(entries, snapshot_entries(snap)?)
}

pub(crate) fn replace_memory_entries(
    entries: &SharedEntries,
    replacement: Vec<Expr>,
) -> Result<()> {
    *lock_entries(entries, "memory entries")? = replacement;
    Ok(())
}

pub(crate) fn snapshot_entries(snap: Expr) -> Result<Vec<Expr>> {
    match snap {
        Expr::Nil => Ok(Vec::new()),
        Expr::List(items) | Expr::Vector(items) => Ok(items),
        _ => Err(Error::TypeMismatch {
            expected: "memory snapshot list",
            found: "non-list",
        }),
    }
}

pub(crate) fn value_to_snapshot_expr(cx: &mut Cx, value: Value) -> Result<Expr> {
    value.object().as_expr(cx)
}

fn query_terms(expr: &Expr) -> Vec<String> {
    if let Expr::List(items) | Expr::Vector(items) = expr
        && let [Expr::Symbol(symbol), payload] = items.as_slice()
        && symbol.name.as_ref() == "query"
    {
        return query_terms(payload);
    }
    let mut out = Vec::new();
    collect_query_terms(expr, &mut out);
    out.retain(|term| !term.is_empty());
    out
}

fn collect_query_terms(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::String(text) => out.extend(text.split_whitespace().map(|term| term.to_lowercase())),
        Expr::Symbol(symbol) => out.push(symbol.to_string().to_lowercase()),
        Expr::Local(symbol) => out.push(symbol.to_string().to_lowercase()),
        Expr::List(items) | Expr::Vector(items) | Expr::Set(items) | Expr::Block(items) => {
            for item in items {
                collect_query_terms(item, out);
            }
        }
        Expr::Map(entries) => {
            for (key, value) in entries {
                collect_query_terms(key, out);
                collect_query_terms(value, out);
            }
        }
        Expr::Number(number) => out.push(number.canonical.to_lowercase()),
        Expr::Bool(value) => out.push(value.to_string()),
        Expr::Bytes(bytes) => out.push(format!("{bytes:?}").to_lowercase()),
        Expr::Nil => {}
        Expr::Call { operator, args } => {
            collect_query_terms(operator, out);
            for arg in args {
                collect_query_terms(arg, out);
            }
        }
        Expr::Infix {
            operator,
            left,
            right,
        } => {
            out.push(operator.to_string().to_lowercase());
            collect_query_terms(left, out);
            collect_query_terms(right, out);
        }
        Expr::Prefix { operator, arg } | Expr::Postfix { operator, arg } => {
            out.push(operator.to_string().to_lowercase());
            collect_query_terms(arg, out);
        }
        Expr::Quote { expr, .. } => collect_query_terms(expr, out),
        Expr::Extension { tag, payload } => {
            out.push(tag.to_string().to_lowercase());
            collect_query_terms(payload, out);
        }
        Expr::Annotated { expr, annotations } => {
            collect_query_terms(expr, out);
            for (key, value) in annotations {
                out.push(key.to_string().to_lowercase());
                collect_query_terms(value, out);
            }
        }
    }
}

fn memory_entry_matches(expr: &Expr, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let haystack = flatten_expr_text(expr);
    terms.iter().all(|term| haystack.contains(term))
}

pub(crate) fn flatten_expr_text(expr: &Expr) -> String {
    let mut out = Vec::new();
    collect_query_terms(expr, &mut out);
    out.join(" ")
}

pub(crate) fn append_memory_log(path: &Path, expr: &Expr) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(io_error)?;
    let payload = encode_frame(expr)?.0;
    let len = u64::try_from(payload.len())
        .map_err(|_| Error::HostError("memory log payload too large".to_owned()))?;
    file.write_all(&len.to_le_bytes()).map_err(io_error)?;
    file.write_all(&payload).map_err(io_error)
}

pub(crate) fn rewrite_memory_log(path: &Path, entries: &[Expr]) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    let mut file = File::create(path).map_err(io_error)?;
    for entry in entries {
        let payload = encode_frame(entry)?.0;
        let len = u64::try_from(payload.len())
            .map_err(|_| Error::HostError("memory log payload too large".to_owned()))?;
        file.write_all(&len.to_le_bytes()).map_err(io_error)?;
        file.write_all(&payload).map_err(io_error)?;
    }
    Ok(())
}

pub(crate) fn load_memory_log(path: &Path) -> Result<Vec<Expr>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(io_error)?
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    let mut offset = 0usize;
    let mut entries = Vec::new();
    while offset < bytes.len() {
        if bytes.len().saturating_sub(offset) < 8 {
            return Err(Error::HostError(format!(
                "memory log {} ended with a partial length header",
                path.display()
            )));
        }
        let len = u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        offset += 8;
        let len = usize::try_from(len)
            .map_err(|_| Error::HostError("memory log record length exceeds usize".to_owned()))?;
        if bytes.len().saturating_sub(offset) < len {
            return Err(Error::HostError(format!(
                "memory log {} ended with a partial record",
                path.display()
            )));
        }
        let (_, expr) = decode_frame(sim_kernel::CodecId(0), &bytes[offset..offset + len])?;
        entries.push(expr);
        offset += len;
    }
    Ok(entries)
}

pub(crate) fn lock_entries<'a>(
    entries: &'a SharedEntries,
    label: &'static str,
) -> Result<std::sync::MutexGuard<'a, Vec<Expr>>> {
    entries.lock().map_err(|_| Error::PoisonedLock(label))
}

pub(crate) fn io_error(err: std::io::Error) -> Error {
    Error::host_io(err)
}
