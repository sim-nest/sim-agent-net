use super::super::store::snapshot_entries;
use super::types::{
    BlackboardMemory, FileMemory, PersonaMemory, VectorMemory, WorkingMemory,
    resolve_memory_backend,
};
use sim_kernel::{Args, CapabilityName, Cx, Error, Result, Value};
use std::sync::Arc;

use crate::{FILE_WRITE_CAPABILITY, parse_component_options, path_option};

pub(crate) fn memory_working_value(cx: &mut Cx, args: Args) -> Result<Value> {
    if !args.values().is_empty() {
        return Err(Error::Eval(
            "memory/working expects no arguments".to_owned(),
        ));
    }
    cx.factory()
        .opaque(Arc::new(WorkingMemory::new(crate::installed_codecs(cx))))
}

pub(crate) fn memory_file_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [path] = args.values() else {
        return Err(Error::Eval(
            "memory/file expects exactly one path argument".to_owned(),
        ));
    };
    cx.require(&CapabilityName::new(FILE_WRITE_CAPABILITY))?;
    let path = crate::string_from_value(cx, path.clone(), "memory/file expects a string path")?;
    cx.factory().opaque(Arc::new(FileMemory::open(
        path,
        crate::installed_codecs(cx),
    )?))
}

pub(crate) fn memory_blackboard_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let board = match args.values() {
        [] => "ephemeral".to_owned(),
        [value] => crate::stringish_from_value(
            cx,
            value.clone(),
            "memory/blackboard expects a symbol or string name",
        )?,
        _ => {
            return Err(Error::Eval(
                "memory/blackboard expects at most one board name".to_owned(),
            ));
        }
    };
    cx.factory().opaque(Arc::new(BlackboardMemory::new(
        board,
        crate::installed_codecs(cx),
    )))
}

pub(crate) fn memory_vector_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "memory/vector")?;
    let path = path_option(cx, &options, "path")?;
    if path.is_some() {
        cx.require(&CapabilityName::new(FILE_WRITE_CAPABILITY))?;
    }
    cx.factory().opaque(Arc::new(VectorMemory::open(
        path,
        crate::installed_codecs(cx),
    )?))
}

pub(crate) fn memory_persona_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "memory/persona")?;
    let path = path_option(cx, &options, "path")?;
    if path.is_some() {
        cx.require(&CapabilityName::new(FILE_WRITE_CAPABILITY))?;
    }
    cx.factory().opaque(Arc::new(PersonaMemory::open(
        path,
        crate::installed_codecs(cx),
    )?))
}

pub(crate) fn memory_append_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [memory, msg] = args.values() else {
        return Err(Error::Eval(
            "memory/append expects a memory and one message".to_owned(),
        ));
    };
    resolve_memory_backend(memory)?.append(cx, msg.clone())?;
    cx.factory().nil()
}

pub(crate) fn memory_recent_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [memory, count] = args.values() else {
        return Err(Error::Eval(
            "memory/recent expects a memory and a count".to_owned(),
        ));
    };
    let count = crate::u32_from_value(cx, count.clone(), "memory/recent expects an integer count")?;
    let recent = resolve_memory_backend(memory)?.recent(cx, count)?;
    cx.factory().list(recent)
}

pub(crate) fn memory_scan_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let memory = match args.values() {
        [memory] => memory,
        [memory, _count] => memory,
        _ => {
            return Err(Error::Eval(
                "memory/scan expects a memory and optional count".to_owned(),
            ));
        }
    };
    let entries = if let [_, count] = args.values() {
        let count =
            crate::u32_from_value(cx, count.clone(), "memory/scan expects an integer count")?;
        resolve_memory_backend(memory)?.recent(cx, count)?
    } else {
        let snapshot = resolve_memory_backend(memory)?.snapshot(cx)?;
        snapshot_entries(snapshot)?
            .into_iter()
            .map(|expr| crate::value_from_expr(cx, &expr))
            .collect::<Result<Vec<_>>>()?
    };
    cx.factory().list(entries)
}

pub(crate) fn memory_search_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [memory, query, k] = args.values() else {
        return Err(Error::Eval(
            "memory/search expects a memory, query, and count".to_owned(),
        ));
    };
    let k = crate::u32_from_value(cx, k.clone(), "memory/search expects an integer count")?;
    let query = query.object().as_expr(cx)?;
    let results = resolve_memory_backend(memory)?.search(cx, query, k)?;
    cx.factory().list(results)
}

pub(crate) fn memory_snapshot_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [memory] = args.values() else {
        return Err(Error::Eval(
            "memory/snapshot expects exactly one memory".to_owned(),
        ));
    };
    let snapshot = resolve_memory_backend(memory)?.snapshot(cx)?;
    cx.factory().expr(snapshot)
}

pub(crate) fn memory_restore_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [memory, snapshot] = args.values() else {
        return Err(Error::Eval(
            "memory/restore expects a memory and a snapshot".to_owned(),
        ));
    };
    let snapshot = snapshot.object().as_expr(cx)?;
    resolve_memory_backend(memory)?.restore(cx, snapshot)?;
    cx.factory().nil()
}
