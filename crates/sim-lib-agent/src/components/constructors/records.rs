use super::super::{
    model::{
        AgentComponent, ComponentBackend, RecorderBackend, VoiceBackend, component_value,
        number_expr,
    },
    options::{parse_component_options, path_option, string_option},
};
use crate::{
    ComponentKind, VOICE_CAPABILITY,
    memory::{SharedEntries, load_memory_log},
    util::installed_codecs,
};
use sim_kernel::{Args, CapabilityName, Cx, Error, Expr, Result, Symbol, Value};
use sim_lib_server::ServerAddress;
use std::sync::{Arc, Mutex};

fn recorder_entries(path: &Option<std::path::PathBuf>) -> Result<SharedEntries> {
    let entries = match path {
        Some(path) => load_memory_log(path)?,
        None => Vec::new(),
    };
    Ok(Arc::new(Mutex::new(entries)))
}

fn entries_len(entries: &SharedEntries) -> Result<u32> {
    Ok(u32::try_from(
        entries
            .lock()
            .map_err(|_| Error::PoisonedLock("recorder entries"))?
            .len(),
    )
    .unwrap_or(u32::MAX))
}

pub(crate) fn recorder_journal_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "recorder/journal")?;
    let path = path_option(cx, &options, "path")?;
    let entries = recorder_entries(&path)?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("recorder", "journal"),
            kind: ComponentKind::Recorder,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![
                (
                    Symbol::new("path"),
                    path.as_ref()
                        .map(|p| Expr::String(p.display().to_string()))
                        .unwrap_or(Expr::Nil),
                ),
                (Symbol::new("entries"), number_expr(entries_len(&entries)?)),
            ],
            backend: ComponentBackend::Recorder(RecorderBackend::Journal { path, entries }),
        },
    )
}

pub(crate) fn recorder_audit_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "recorder/audit")?;
    let path = path_option(cx, &options, "path")?;
    let entries = recorder_entries(&path)?;
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("recorder", "audit"),
            kind: ComponentKind::Recorder,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![
                (
                    Symbol::new("path"),
                    path.as_ref()
                        .map(|p| Expr::String(p.display().to_string()))
                        .unwrap_or(Expr::Nil),
                ),
                (Symbol::new("entries"), number_expr(entries_len(&entries)?)),
            ],
            backend: ComponentBackend::Recorder(RecorderBackend::Audit { path, entries }),
        },
    )
}

pub(crate) fn recorder_prometheus_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "recorder/prometheus")?;
    let namespace = string_option(cx, &options, "namespace", "sim_agent")?;
    let entries = Arc::new(Mutex::new(Vec::new()));
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("recorder", "prometheus"),
            kind: ComponentKind::Recorder,
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![(Symbol::new("namespace"), Expr::String(namespace.clone()))],
            backend: ComponentBackend::Recorder(RecorderBackend::Prometheus { namespace, entries }),
        },
    )
}

pub(crate) fn voice_tts_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "voice/tts")?;
    let voice = string_option(cx, &options, "voice", "default")?;
    let command = options
        .get("command")
        .map(|value| {
            crate::util::stringish_from_value(cx, value.clone(), "voice/tts :command expects text")
        })
        .transpose()?;
    let capabilities = command
        .as_ref()
        .map(|_| vec![CapabilityName::new(VOICE_CAPABILITY)])
        .unwrap_or_default();
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("voice", "tts"),
            kind: ComponentKind::Voice,
            capabilities,
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![
                (Symbol::new("voice"), Expr::String(voice.clone())),
                (
                    Symbol::new("command"),
                    command.clone().map(Expr::String).unwrap_or(Expr::Nil),
                ),
            ],
            backend: ComponentBackend::Voice(VoiceBackend::Tts { voice, command }),
        },
    )
}

pub(crate) fn voice_stt_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let options = parse_component_options(cx, args, "voice/stt")?;
    let locale = string_option(cx, &options, "locale", "en-US")?;
    let command = options
        .get("command")
        .map(|value| {
            crate::util::stringish_from_value(cx, value.clone(), "voice/stt :command expects text")
        })
        .transpose()?;
    let capabilities = command
        .as_ref()
        .map(|_| vec![CapabilityName::new(VOICE_CAPABILITY)])
        .unwrap_or_default();
    component_value(
        cx,
        AgentComponent {
            symbol: Symbol::qualified("voice", "stt"),
            kind: ComponentKind::Voice,
            capabilities,
            address: ServerAddress::Local,
            codecs: installed_codecs(cx),
            spec: vec![
                (Symbol::new("locale"), Expr::String(locale.clone())),
                (
                    Symbol::new("command"),
                    command.clone().map(Expr::String).unwrap_or(Expr::Nil),
                ),
            ],
            backend: ComponentBackend::Voice(VoiceBackend::Stt { locale, command }),
        },
    )
}
