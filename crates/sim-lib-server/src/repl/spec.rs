use std::path::PathBuf;

use sim_kernel::{CapabilityName, Error, Expr, Result, Symbol};

use crate::{keyword, line_driver_factories};

use super::drivers::{BufferDriver, ExternalDriver, StdioLineDriver};

const AGENT_DRIVE_CAPABILITY: &str = "agent-drive";

/// Source of REPL input lines and sink for rendered output.
pub trait LineDriver: Send + Sync {
    /// Reads the next input line, showing `prompt`; returns `None` at end of input.
    fn read_line(&mut self, cx: &mut sim_kernel::Cx, prompt: &str) -> Result<Option<String>>;
    /// Writes rendered `output` to the driver.
    fn write_output(&mut self, cx: &mut sim_kernel::Cx, output: &str) -> Result<()>;
    /// Reports whether the driver reads multi-line input. Defaults to `false`.
    fn supports_multiline(&self) -> bool {
        false
    }
    /// Returns the capabilities the driver requires. Defaults to none.
    fn capabilities(&self) -> &[CapabilityName] {
        &[]
    }
}

/// Declarative description of a REPL line driver, resolved into a concrete
/// [`LineDriver`] by [`DriverSpec::create_driver`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriverSpec {
    /// Single-line stdio driver.
    Line,
    /// Multi-line stdio driver.
    Multiline,
    /// External process driver invoking `cmd`.
    External {
        /// Command line to spawn as the driver.
        cmd: String,
    },
    /// Buffer-file driver reading from `path`.
    Buffer {
        /// Path to the buffer file.
        path: PathBuf,
        /// Symbol naming the trigger event (for example `save`).
        on: Symbol,
    },
    /// A driver named in the spec but not built in; resolved from the registry
    /// at creation time or reported as unavailable.
    Unavailable {
        /// The original driver spec expression.
        spec: Expr,
    },
}

impl DriverSpec {
    /// Parses a driver spec from `expr`: a driver symbol (`line`, `multiline`,
    /// ...) or a `(kind ...)` option list. Returns an error for malformed specs.
    pub fn from_expr(expr: &Expr) -> Result<Self> {
        match expr {
            Expr::Symbol(symbol) if symbol.name.as_ref() == "line" => Ok(Self::Line),
            Expr::Symbol(symbol) if symbol.name.as_ref() == "multiline" => Ok(Self::Multiline),
            Expr::Symbol(symbol)
                if symbol.name.as_ref() == "readline"
                    || symbol.name.as_ref() == "browser"
                    || symbol.name.as_ref() == "agent" =>
            {
                Ok(Self::Unavailable { spec: expr.clone() })
            }
            Expr::List(items) | Expr::Vector(items) => Self::from_items(items),
            _ => Err(Error::Eval(
                "server/repl :driver expects a driver symbol or spec list".to_owned(),
            )),
        }
    }

    fn from_items(items: &[Expr]) -> Result<Self> {
        let Some(Expr::Symbol(kind)) = items.first() else {
            return Err(Error::Eval(
                "server/repl :driver list must start with a symbol".to_owned(),
            ));
        };
        match kind.name.as_ref() {
            "external" => {
                let cmd = find_string_option(
                    items,
                    "cmd",
                    "external driver requires :cmd",
                    "external :cmd expects a string",
                )?;
                Ok(Self::External { cmd })
            }
            "buffer" => {
                let path = PathBuf::from(find_string_option(
                    items,
                    "path",
                    "buffer driver requires :path",
                    "buffer :path expects a string",
                )?);
                let on = find_symbol_option(items, "on", "buffer :on expects a symbol")?
                    .unwrap_or_else(|| Symbol::new("save"));
                Ok(Self::Buffer { path, on })
            }
            "readline" | "browser" | "agent" => Ok(Self::Unavailable {
                spec: Expr::List(items.to_vec()),
            }),
            other => Err(Error::Eval(format!(
                "server/repl: unknown driver kind {other}"
            ))),
        }
    }

    /// Renders this spec back into its `Expr` form, the inverse of
    /// [`DriverSpec::from_expr`].
    pub fn as_expr(&self) -> Expr {
        match self {
            Self::Line => Expr::Symbol(Symbol::new("line")),
            Self::Multiline => Expr::Symbol(Symbol::new("multiline")),
            Self::External { cmd } => Expr::List(vec![
                Expr::Symbol(Symbol::new("external")),
                Expr::Symbol(Symbol::new(":cmd")),
                Expr::String(cmd.clone()),
            ]),
            Self::Buffer { path, on } => Expr::List(vec![
                Expr::Symbol(Symbol::new("buffer")),
                Expr::Symbol(Symbol::new(":path")),
                Expr::String(path.display().to_string()),
                Expr::Symbol(Symbol::new(":on")),
                Expr::Quote {
                    mode: sim_kernel::QuoteMode::Quote,
                    expr: Box::new(Expr::Symbol(on.clone())),
                },
            ]),
            Self::Unavailable { spec } => spec.clone(),
        }
    }

    /// Builds the concrete [`LineDriver`] for this spec, consulting the driver
    /// registry for [`DriverSpec::Unavailable`] specs and erroring if none is found.
    pub fn create_driver(&self, cx: &mut sim_kernel::Cx) -> Result<Box<dyn LineDriver>> {
        match self {
            Self::Line => Ok(Box::new(StdioLineDriver::new(false))),
            Self::Multiline => Ok(Box::new(StdioLineDriver::new(true))),
            Self::External { cmd } => Ok(Box::new(ExternalDriver::new(cmd.clone()))),
            Self::Buffer { path, on } => {
                if on.name.as_ref() != "save" {
                    return Err(Error::Eval(format!(
                        "buffer driver does not support :on {} yet",
                        on
                    )));
                }
                Ok(Box::new(BufferDriver::new(path.clone())))
            }
            Self::Unavailable { spec } => {
                if let Some(driver) = lookup_registered_driver(cx, spec)? {
                    return Ok(driver);
                }
                Err(unavailable_driver_error(spec))
            }
        }
    }

    /// Returns the capabilities required to use this driver spec.
    pub fn required_capabilities(&self) -> Vec<CapabilityName> {
        match self {
            Self::Unavailable { spec } => unavailable_driver_capabilities(spec),
            _ => Vec::new(),
        }
    }
}

fn find_string_option(
    items: &[Expr],
    name: &str,
    missing: &'static str,
    wrong: &'static str,
) -> Result<String> {
    let mut iter = items.iter().skip(1);
    while let Some(key_expr) = iter.next() {
        let Some(value) = iter.next() else {
            return Err(Error::Eval(
                "driver options must be key/value pairs".to_owned(),
            ));
        };
        if keyword(key_expr)? == name {
            return match value {
                Expr::String(text) => Ok(text.clone()),
                _ => Err(Error::Eval(wrong.to_owned())),
            };
        }
    }
    Err(Error::Eval(missing.to_owned()))
}

fn find_symbol_option(items: &[Expr], name: &str, wrong: &'static str) -> Result<Option<Symbol>> {
    let mut iter = items.iter().skip(1);
    while let Some(key_expr) = iter.next() {
        let Some(value) = iter.next() else {
            return Err(Error::Eval(
                "driver options must be key/value pairs".to_owned(),
            ));
        };
        if keyword(key_expr)? == name {
            return match value {
                Expr::Symbol(symbol) => Ok(Some(symbol.clone())),
                Expr::Quote { expr, .. } => match expr.as_ref() {
                    Expr::Symbol(symbol) => Ok(Some(symbol.clone())),
                    _ => Err(Error::Eval(wrong.to_owned())),
                },
                _ => Err(Error::Eval(wrong.to_owned())),
            };
        }
    }
    Ok(None)
}

fn unavailable_driver_capabilities(spec: &Expr) -> Vec<CapabilityName> {
    match spec {
        Expr::Symbol(symbol) if symbol.name.as_ref() == "agent" => {
            vec![CapabilityName::new(AGENT_DRIVE_CAPABILITY)]
        }
        Expr::List(items) | Expr::Vector(items) if matches!(items.first(), Some(Expr::Symbol(symbol)) if symbol.name.as_ref() == "agent") =>
        {
            vec![CapabilityName::new(AGENT_DRIVE_CAPABILITY)]
        }
        _ => Vec::new(),
    }
}

fn lookup_registered_driver(
    cx: &mut sim_kernel::Cx,
    spec: &Expr,
) -> Result<Option<Box<dyn LineDriver>>> {
    let Some(name) = unavailable_driver_name(spec) else {
        return Ok(None);
    };
    let factory = {
        let factories = line_driver_factories()
            .lock()
            .map_err(|_| Error::HostError("line driver registry mutex poisoned".to_owned()))?;
        factories.get(name).copied()
    };
    let Some(factory) = factory else {
        return Ok(None);
    };
    factory(cx, spec)
}

fn unavailable_driver_name(spec: &Expr) -> Option<&str> {
    match spec {
        Expr::Symbol(symbol) => Some(symbol.name.as_ref()),
        Expr::List(items) | Expr::Vector(items) => match items.first() {
            Some(Expr::Symbol(symbol)) => Some(symbol.name.as_ref()),
            _ => None,
        },
        _ => None,
    }
}

fn unavailable_driver_error(spec: &Expr) -> Error {
    match unavailable_driver_name(spec) {
        Some("browser") => Error::Eval("browser driver not available".to_owned()),
        Some("readline") => Error::Eval("readline driver not available".to_owned()),
        _ => Error::Eval("driver not available".to_owned()),
    }
}
