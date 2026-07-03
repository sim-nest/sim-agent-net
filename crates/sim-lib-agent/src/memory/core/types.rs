use super::advanced::{PersonaState, VectorEntry};
use super::requests::answer_memory_request;
use sim_kernel::{
    CapabilityName, ClassRef, Cx, Error, EvalReply, Expr, Object, ObjectCompat, Result, Symbol,
    Value,
};
use sim_lib_server::{
    EvalSite, FrameKind, ServerAddress, ServerFrame, eval_request_from_frame,
    server_frame_from_reply,
};
use std::{
    any::Any,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use crate::{Component, ComponentKind, FILE_WRITE_CAPABILITY};

use super::super::store::{
    append_memory_log, load_memory_log, memory_entries_append, memory_entries_recent,
    memory_entries_restore, memory_entries_search, memory_entries_snapshot, replace_memory_entries,
    rewrite_memory_log, shared_blackboard_entries, snapshot_entries, value_to_snapshot_expr,
};

/// Category of a memory backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    /// In-process working memory that does not persist.
    Working,
    /// File-backed episodic log.
    Episodic,
    /// Vector store for similarity search.
    Vector,
    /// Shared blackboard memory addressed by board name.
    Blackboard,
    /// Persona state memory.
    Persona,
    /// A caller-defined kind tagged by symbol.
    Custom(Symbol),
}

impl MemoryKind {
    /// Returns the canonical symbol naming this kind.
    pub fn as_symbol(&self) -> Symbol {
        match self {
            Self::Working => Symbol::new("working"),
            Self::Episodic => Symbol::new("episodic"),
            Self::Vector => Symbol::new("vector"),
            Self::Blackboard => Symbol::new("blackboard"),
            Self::Persona => Symbol::new("persona"),
            Self::Custom(symbol) => symbol.clone(),
        }
    }
}

/// Behavior shared by every memory backend: append, query, and snapshot/restore.
pub trait MemoryBackend: Component {
    /// Appends one message to the backend.
    fn append(&self, cx: &mut Cx, msg: Value) -> Result<()>;
    /// Returns up to `count` most recent entries.
    fn recent(&self, cx: &mut Cx, count: u32) -> Result<Vec<Value>>;
    /// Returns up to `k` entries matching `query`.
    fn search(&self, cx: &mut Cx, query: Expr, k: u32) -> Result<Vec<Value>>;
    /// Captures the full backend state as an expression.
    fn snapshot(&self, cx: &mut Cx) -> Result<Expr>;
    /// Replaces the backend state from a prior snapshot.
    fn restore(&self, cx: &mut Cx, snap: Expr) -> Result<()>;
}

/// In-process working memory holding entries in a shared buffer.
#[derive(Clone)]
pub struct WorkingMemory {
    pub(crate) symbol: Symbol,
    pub(crate) capabilities: Vec<CapabilityName>,
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
    pub(crate) entries: Arc<Mutex<Vec<Expr>>>,
}

/// File-backed episodic memory that persists entries to a log on disk.
#[derive(Clone)]
pub struct FileMemory {
    pub(crate) symbol: Symbol,
    pub(crate) capabilities: Vec<CapabilityName>,
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
    pub(crate) path: PathBuf,
    pub(crate) entries: Arc<Mutex<Vec<Expr>>>,
}

/// Memory whose entries are shared across agents on a named blackboard.
#[derive(Clone)]
pub struct BlackboardMemory {
    pub(crate) symbol: Symbol,
    pub(crate) board: String,
    pub(crate) capabilities: Vec<CapabilityName>,
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
    pub(crate) entries: Arc<Mutex<Vec<Expr>>>,
}

/// Vector memory supporting similarity search, optionally persisted to a path.
#[derive(Clone)]
pub struct VectorMemory {
    pub(crate) symbol: Symbol,
    pub(crate) capabilities: Vec<CapabilityName>,
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
    pub(crate) path: Option<PathBuf>,
    pub(super) entries: Arc<Mutex<Vec<VectorEntry>>>,
}

/// Memory holding evolving persona state, optionally persisted to a path.
#[derive(Clone)]
pub struct PersonaMemory {
    pub(crate) symbol: Symbol,
    pub(crate) capabilities: Vec<CapabilityName>,
    pub(crate) address: ServerAddress,
    pub(crate) codecs: Vec<Symbol>,
    pub(crate) path: Option<PathBuf>,
    pub(super) state: Arc<Mutex<PersonaState>>,
}

impl WorkingMemory {
    pub(crate) fn new(codecs: Vec<Symbol>) -> Self {
        Self {
            symbol: Symbol::qualified("memory", "working"),
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs,
            entries: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl FileMemory {
    pub(crate) fn open(path: impl Into<PathBuf>, codecs: Vec<Symbol>) -> Result<Self> {
        let path = path.into();
        let entries = load_memory_log(&path)?;
        Ok(Self {
            symbol: Symbol::qualified("memory", "file"),
            capabilities: vec![CapabilityName::new(FILE_WRITE_CAPABILITY)],
            address: ServerAddress::Local,
            codecs,
            path,
            entries: Arc::new(Mutex::new(entries)),
        })
    }
}

impl BlackboardMemory {
    pub(crate) fn new(board: String, codecs: Vec<Symbol>) -> Self {
        Self {
            symbol: Symbol::qualified("memory", "blackboard"),
            board: board.clone(),
            capabilities: Vec::new(),
            address: ServerAddress::Local,
            codecs,
            entries: shared_blackboard_entries(&board),
        }
    }
}

fn optional_path_entry(cx: &mut Cx, path: &Option<PathBuf>) -> Result<Vec<(Symbol, Value)>> {
    match path {
        Some(path) => Ok(vec![(
            Symbol::new("path"),
            cx.factory().string(path.display().to_string())?,
        )]),
        None => Ok(Vec::new()),
    }
}

macro_rules! impl_memory_object {
    ($ty:ty, $display:expr, $kind:expr, $extra:expr) => {
        impl Object for $ty {
            fn display(&self, _cx: &mut Cx) -> Result<String> {
                Ok($display(self))
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        impl sim_kernel::ObjectCompat for $ty {
            fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
                if let Some(value) = cx
                    .registry()
                    .class_by_symbol(&Symbol::qualified("core", "Table"))
                {
                    return Ok(value.clone());
                }
                cx.factory().class_stub(
                    sim_kernel::CORE_TABLE_CLASS_ID,
                    Symbol::qualified("core", "Table"),
                )
            }
            fn as_expr(&self, cx: &mut Cx) -> Result<Expr> {
                self.as_table(cx)?.object().as_expr(cx)
            }
            fn as_table(&self, cx: &mut Cx) -> Result<Value> {
                let mut entries = vec![
                    (Symbol::new("kind"), cx.factory().symbol($kind.as_symbol())?),
                    (
                        Symbol::new("name"),
                        cx.factory().symbol(self.symbol.clone())?,
                    ),
                    (
                        Symbol::new("capabilities"),
                        cx.factory().list(
                            self.capabilities
                                .iter()
                                .map(|capability| {
                                    cx.factory().string(capability.as_str().to_owned())
                                })
                                .collect::<Result<Vec<_>>>()?,
                        )?,
                    ),
                ];
                entries.extend($extra(self, cx)?);
                cx.factory().table(entries)
            }
        }

        impl EvalSite for $ty {
            fn site_kind(&self) -> &'static str {
                "memory"
            }

            fn address(&self) -> &ServerAddress {
                &self.address
            }

            fn codecs(&self) -> &[Symbol] {
                &self.codecs
            }

            fn answer(&self, cx: &mut Cx, frame: ServerFrame) -> Result<ServerFrame> {
                if frame.kind != FrameKind::Request {
                    return Err(Error::Eval(format!(
                        "memory {} cannot answer frame kind {}",
                        self.symbol,
                        frame.kind.as_symbol()
                    )));
                }
                let consistency = frame.envelope.consistency;
                let request = eval_request_from_frame(cx, &frame)?;
                let value = answer_memory_request(self, cx, request.expr)?;
                let diagnostics = cx.take_diagnostics();
                server_frame_from_reply(
                    cx,
                    &frame.codec,
                    EvalReply {
                        value,
                        diagnostics,
                        trace: None,
                    },
                    consistency,
                )
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }

        impl Component for $ty {
            fn kind(&self) -> ComponentKind {
                ComponentKind::Memory
            }

            fn name(&self) -> &Symbol {
                &self.symbol
            }

            fn capabilities(&self) -> &[CapabilityName] {
                &self.capabilities
            }

            fn reflect(&self, cx: &mut Cx) -> Result<Expr> {
                self.as_table(cx)?.object().as_expr(cx)
            }
        }
    };
}

impl_memory_object!(
    WorkingMemory,
    |_memory: &WorkingMemory| "#<memory working>".to_owned(),
    MemoryKind::Working,
    |_memory: &WorkingMemory, _cx: &mut Cx| -> Result<Vec<(Symbol, Value)>> { Ok(Vec::new()) }
);

impl_memory_object!(
    FileMemory,
    |memory: &FileMemory| format!("#<memory file {}>", memory.path.display()),
    MemoryKind::Episodic,
    |memory: &FileMemory, cx: &mut Cx| -> Result<Vec<(Symbol, Value)>> {
        Ok(vec![(
            Symbol::new("path"),
            cx.factory().string(memory.path.display().to_string())?,
        )])
    }
);

impl_memory_object!(
    BlackboardMemory,
    |memory: &BlackboardMemory| format!("#<memory blackboard {}>", memory.board),
    MemoryKind::Blackboard,
    |memory: &BlackboardMemory, cx: &mut Cx| -> Result<Vec<(Symbol, Value)>> {
        Ok(vec![(
            Symbol::new("board"),
            cx.factory().string(memory.board.clone())?,
        )])
    }
);

impl_memory_object!(
    VectorMemory,
    |memory: &VectorMemory| match &memory.path {
        Some(path) => format!("#<memory vector {}>", path.display()),
        None => "#<memory vector>".to_owned(),
    },
    MemoryKind::Vector,
    |memory: &VectorMemory, cx: &mut Cx| optional_path_entry(cx, &memory.path)
);

impl_memory_object!(
    PersonaMemory,
    |memory: &PersonaMemory| match &memory.path {
        Some(path) => format!("#<memory persona {}>", path.display()),
        None => "#<memory persona>".to_owned(),
    },
    MemoryKind::Persona,
    |memory: &PersonaMemory, cx: &mut Cx| optional_path_entry(cx, &memory.path)
);

impl MemoryBackend for WorkingMemory {
    fn append(&self, cx: &mut Cx, msg: Value) -> Result<()> {
        memory_entries_append(&self.entries, value_to_snapshot_expr(cx, msg)?)
    }

    fn recent(&self, cx: &mut Cx, count: u32) -> Result<Vec<Value>> {
        memory_entries_recent(cx, &self.entries, count)
    }

    fn search(&self, cx: &mut Cx, query: Expr, k: u32) -> Result<Vec<Value>> {
        memory_entries_search(cx, &self.entries, query, k)
    }

    fn snapshot(&self, _cx: &mut Cx) -> Result<Expr> {
        memory_entries_snapshot(&self.entries)
    }

    fn restore(&self, _cx: &mut Cx, snap: Expr) -> Result<()> {
        memory_entries_restore(&self.entries, snap)
    }
}

impl MemoryBackend for FileMemory {
    fn append(&self, cx: &mut Cx, msg: Value) -> Result<()> {
        cx.require(&CapabilityName::new(FILE_WRITE_CAPABILITY))?;
        let expr = value_to_snapshot_expr(cx, msg)?;
        memory_entries_append(&self.entries, expr.clone())?;
        append_memory_log(&self.path, &expr)
    }

    fn recent(&self, cx: &mut Cx, count: u32) -> Result<Vec<Value>> {
        memory_entries_recent(cx, &self.entries, count)
    }

    fn search(&self, cx: &mut Cx, query: Expr, k: u32) -> Result<Vec<Value>> {
        memory_entries_search(cx, &self.entries, query, k)
    }

    fn snapshot(&self, _cx: &mut Cx) -> Result<Expr> {
        memory_entries_snapshot(&self.entries)
    }

    fn restore(&self, cx: &mut Cx, snap: Expr) -> Result<()> {
        cx.require(&CapabilityName::new(FILE_WRITE_CAPABILITY))?;
        let entries = snapshot_entries(snap)?;
        rewrite_memory_log(&self.path, &entries)?;
        replace_memory_entries(&self.entries, entries)
    }
}

impl MemoryBackend for BlackboardMemory {
    fn append(&self, cx: &mut Cx, msg: Value) -> Result<()> {
        memory_entries_append(&self.entries, value_to_snapshot_expr(cx, msg)?)
    }

    fn recent(&self, cx: &mut Cx, count: u32) -> Result<Vec<Value>> {
        memory_entries_recent(cx, &self.entries, count)
    }

    fn search(&self, cx: &mut Cx, query: Expr, k: u32) -> Result<Vec<Value>> {
        memory_entries_search(cx, &self.entries, query, k)
    }

    fn snapshot(&self, _cx: &mut Cx) -> Result<Expr> {
        memory_entries_snapshot(&self.entries)
    }

    fn restore(&self, _cx: &mut Cx, snap: Expr) -> Result<()> {
        memory_entries_restore(&self.entries, snap)
    }
}

pub(crate) fn resolve_memory_backend(value: &Value) -> Result<&dyn MemoryBackend> {
    if let Some(memory) = value.object().downcast_ref::<WorkingMemory>() {
        return Ok(memory);
    }
    if let Some(memory) = value.object().downcast_ref::<FileMemory>() {
        return Ok(memory);
    }
    if let Some(memory) = value.object().downcast_ref::<VectorMemory>() {
        return Ok(memory);
    }
    if let Some(memory) = value.object().downcast_ref::<BlackboardMemory>() {
        return Ok(memory);
    }
    if let Some(memory) = value.object().downcast_ref::<PersonaMemory>() {
        return Ok(memory);
    }
    Err(Error::TypeMismatch {
        expected: "memory",
        found: "non-memory",
    })
}
