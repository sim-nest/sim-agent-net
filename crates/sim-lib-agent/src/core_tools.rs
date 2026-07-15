use std::{any::Any, sync::Arc};

use sim_kernel::{
    Args, CORE_FUNCTION_CLASS_ID, Callable, CapabilityName, ClassRef, Cx, Error, Object, Result,
    ShapeRef, Symbol, Value,
};
use sim_shape::{AnyShape, ListShape, shape_value};

use crate::{
    Tool, ToolSpec, edit_capability, exec_capability, find_capability, fs_read_capability,
    fs_write_capability, install_tool, net_http_capability, symbol_from_value,
};

pub(crate) fn install_core_tools(cx: &mut Cx) -> Result<()> {
    install_tool_group(cx)
}

fn install_tool_group(cx: &mut Cx) -> Result<()> {
    for kind in CoreToolKind::ALL {
        let tool = core_tool(cx, kind)?;
        install_tool(cx, Arc::new(tool))?;
    }
    Ok(())
}

fn core_tool(cx: &mut Cx, kind: CoreToolKind) -> Result<Tool> {
    let function = cx.factory().opaque(Arc::new(CoreToolFn { kind }))?;
    Ok(Tool::local(
        cx,
        ToolSpec {
            symbol: kind.symbol(),
            description: kind.description().to_owned(),
            args_shape: kind.args_shape(),
            result_shape: Some(any_shape(&format!("{}-result", kind.name()))),
            category: Symbol::new("io"),
            capabilities: kind.capabilities(),
            function,
        },
    ))
}

#[derive(Clone, Copy)]
enum CoreToolKind {
    Read,
    Write,
    List,
    Edit,
    Find,
    Exec,
    Fetch,
}

impl CoreToolKind {
    const ALL: [Self; 7] = [
        Self::Read,
        Self::Write,
        Self::List,
        Self::Edit,
        Self::Find,
        Self::Exec,
        Self::Fetch,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::List => "list",
            Self::Edit => "edit",
            Self::Find => "find",
            Self::Exec => "exec",
            Self::Fetch => "fetch",
        }
    }

    fn symbol(self) -> Symbol {
        Symbol::new(self.name())
    }

    fn description(self) -> &'static str {
        match self {
            Self::Read => "Read one table or directory entry as a SIM value.",
            Self::Write => "Write one table or directory entry as a SIM value.",
            Self::List => "List table or directory keys.",
            Self::Edit => "Apply an in-place directory leaf edit through the general dir/edit op.",
            Self::Find => "Search a directory through the general find/grep op.",
            Self::Exec => "Run a bounded host process through the general exec op.",
            Self::Fetch => "Fetch one HTTP directory entry as a SIM value.",
        }
    }

    fn capabilities(self) -> Vec<CapabilityName> {
        match self {
            Self::Read | Self::List => vec![fs_read_capability()],
            Self::Write => vec![fs_write_capability()],
            Self::Edit => vec![fs_read_capability(), edit_capability()],
            Self::Find => vec![fs_read_capability(), find_capability()],
            Self::Exec => vec![exec_capability()],
            Self::Fetch => vec![net_http_capability()],
        }
    }

    fn args_shape(self) -> ShapeRef {
        match self {
            Self::Read | Self::Fetch => list_shape(self.name(), 2),
            Self::Write => list_shape(self.name(), 3),
            Self::List => list_shape(self.name(), 1),
            Self::Edit => list_shape(self.name(), 5),
            Self::Find => list_shape(self.name(), 4),
            Self::Exec => list_shape(self.name(), 2),
        }
    }
}

#[derive(Clone)]
struct CoreToolFn {
    kind: CoreToolKind,
}

impl Object for CoreToolFn {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<function core-tool/{}>", self.kind.name()))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl sim_kernel::ObjectCompat for CoreToolFn {
    fn class(&self, cx: &mut Cx) -> Result<ClassRef> {
        if let Some(value) = cx
            .registry()
            .class_by_symbol(&Symbol::qualified("core", "Function"))
        {
            return Ok(value.clone());
        }
        cx.factory().class_stub(
            CORE_FUNCTION_CLASS_ID,
            Symbol::qualified("core", "Function"),
        )
    }

    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for CoreToolFn {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        match self.kind {
            CoreToolKind::Read => read_value(cx, args),
            CoreToolKind::Write => write_value(cx, args),
            CoreToolKind::List => list_value(cx, args),
            CoreToolKind::Edit => call_general_op(
                cx,
                args,
                &[Symbol::qualified("dir", "edit"), Symbol::new("dir/edit")],
            ),
            CoreToolKind::Find => call_general_op(
                cx,
                args,
                &[Symbol::qualified("find", "grep"), Symbol::new("find/grep")],
            ),
            CoreToolKind::Exec => call_general_op(
                cx,
                args,
                &[Symbol::new("exec"), Symbol::qualified("core", "exec")],
            ),
            CoreToolKind::Fetch => read_value(cx, args),
        }
    }
}

fn read_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [table_value, key_value] = args.values() else {
        return Err(Error::Eval("read expects a table and key".to_owned()));
    };
    let table = table_value
        .object()
        .as_table_impl()
        .ok_or(Error::TypeMismatch {
            expected: "table",
            found: "non-table",
        })?;
    let key = symbol_from_value(cx, key_value.clone(), "read expects a symbol or string key")?;
    table.get(cx, key)
}

fn write_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [table_value, key_value, value] = args.values() else {
        return Err(Error::Eval(
            "write expects a table, key, and value".to_owned(),
        ));
    };
    let table = table_value
        .object()
        .as_table_impl()
        .ok_or(Error::TypeMismatch {
            expected: "table",
            found: "non-table",
        })?;
    let key = symbol_from_value(
        cx,
        key_value.clone(),
        "write expects a symbol or string key",
    )?;
    table.set(cx, key, value.clone())?;
    cx.factory().nil()
}

fn list_value(cx: &mut Cx, args: Args) -> Result<Value> {
    let [table_value] = args.values() else {
        return Err(Error::Eval("list expects one table".to_owned()));
    };
    let table = table_value
        .object()
        .as_table_impl()
        .ok_or(Error::TypeMismatch {
            expected: "table",
            found: "non-table",
        })?;
    let keys = table
        .keys(cx)?
        .into_iter()
        .map(|key| cx.factory().symbol(key))
        .collect::<Result<Vec<_>>>()?;
    cx.factory().list(keys)
}

fn call_general_op(cx: &mut Cx, args: Args, symbols: &[Symbol]) -> Result<Value> {
    let values = args.into_vec();
    let mut last_error = None;
    for symbol in symbols {
        match cx.resolve_function(symbol) {
            Ok(function) => return cx.call_value(function, Args::new(values.clone())),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| Error::UnknownFunction {
        function: Symbol::new("<core-tool-target>"),
    }))
}

fn list_shape(name: &str, len: usize) -> ShapeRef {
    shape_value(
        Symbol::qualified("agent/core", format!("{name}-args")),
        Arc::new(ListShape::new(
            (0..len)
                .map(|_| Arc::new(AnyShape) as Arc<dyn sim_shape::Shape>)
                .collect(),
        )),
    )
}

fn any_shape(name: &str) -> ShapeRef {
    shape_value(
        Symbol::qualified("agent/core", name.to_owned()),
        Arc::new(AnyShape),
    )
}
