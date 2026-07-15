use std::sync::Arc;

use sim_codec_bridge::{BridgeBook, BridgeFramePayload, expr_to_packet, packet_to_expr};
use sim_kernel::{
    AbiVersion, Args, Callable, Cx, Error, Export, Lib, LibManifest, LibTarget, Linker, LoadCx,
    Object, ObjectCompat, Result, Symbol, Value, Version,
};
use sim_shape::{AnyShape, ListShape, Shape, shape_value};

use crate::{bridge_brief, bridge_tx, receipt_packet_for_report, rx_check};

/// Loadable BRIDGE runtime library.
pub struct BridgeLib;

impl Lib for BridgeLib {
    fn manifest(&self) -> LibManifest {
        LibManifest {
            id: manifest_name(),
            version: Version(env!("CARGO_PKG_VERSION").to_owned()),
            abi: AbiVersion { major: 0, minor: 1 },
            target: LibTarget::HostRegistered,
            requires: Vec::new(),
            capabilities: Vec::new(),
            exports: bridge_exports(),
        }
    }

    fn load(&self, cx: &mut LoadCx, linker: &mut Linker<'_>) -> Result<()> {
        for kind in BridgeFunctionKind::ALL {
            let function = BridgeFunction::value(kind);
            linker.function_value(function.symbol(), cx.factory().opaque(function)?)?;
        }
        Ok(())
    }
}

/// Installs the BRIDGE runtime library into a context.
pub fn install_bridge_lib(cx: &mut Cx) -> Result<()> {
    cx.load_lib(&BridgeLib).map(|_| ())
}

/// Manifest symbol for the BRIDGE runtime library.
pub fn manifest_name() -> Symbol {
    Symbol::qualified("sim", "bridge")
}

/// Runtime symbol for `bridge/tx`.
pub fn bridge_tx_symbol() -> Symbol {
    Symbol::qualified("bridge", "tx")
}

/// Runtime symbol for `bridge/rx`.
pub fn bridge_rx_symbol() -> Symbol {
    Symbol::qualified("bridge", "rx")
}

/// Runtime symbol for `bridge/report`.
pub fn bridge_report_symbol() -> Symbol {
    Symbol::qualified("bridge", "report")
}

/// Runtime symbol for `bridge/brief`.
pub fn bridge_brief_symbol() -> Symbol {
    Symbol::qualified("bridge", "brief")
}

fn bridge_exports() -> Vec<Export> {
    BridgeFunctionKind::ALL
        .iter()
        .map(|kind| Export::Function {
            symbol: kind.symbol(),
            function_id: None,
        })
        .collect()
}

/// Runtime callable kind for BRIDGE exports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BridgeFunctionKind {
    /// Build a checked eval request from a packet.
    Tx,
    /// Decode and check a model response packet.
    Rx,
    /// Produce a receive-check report for a packet.
    Report,
    /// Produce a receipt packet for a report.
    Receipt,
    /// Build a BRIEF request packet from one typed frame.
    Brief,
}

impl BridgeFunctionKind {
    /// All exported function kinds.
    pub const ALL: [Self; 5] = [Self::Tx, Self::Rx, Self::Report, Self::Receipt, Self::Brief];

    /// Runtime symbol for this kind.
    pub fn symbol(self) -> Symbol {
        match self {
            Self::Tx => bridge_tx_symbol(),
            Self::Rx => bridge_rx_symbol(),
            Self::Report => bridge_report_symbol(),
            Self::Receipt => crate::receipt_symbol(),
            Self::Brief => bridge_brief_symbol(),
        }
    }
}

/// Runtime callable implementing one BRIDGE export.
#[derive(Clone)]
pub struct BridgeFunction {
    kind: BridgeFunctionKind,
}

impl BridgeFunction {
    /// Builds a function object for `kind`.
    pub fn new(kind: BridgeFunctionKind) -> Self {
        Self { kind }
    }

    /// Returns this function's runtime symbol.
    pub fn symbol(&self) -> Symbol {
        self.kind.symbol()
    }

    /// Builds a shared function object for `kind`.
    pub fn value(kind: BridgeFunctionKind) -> Arc<Self> {
        Arc::new(Self::new(kind))
    }
}

impl Object for BridgeFunction {
    fn display(&self, _cx: &mut Cx) -> Result<String> {
        Ok(format!("#<function {}>", self.symbol()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ObjectCompat for BridgeFunction {
    fn as_callable(&self) -> Option<&dyn Callable> {
        Some(self)
    }
}

impl Callable for BridgeFunction {
    fn call(&self, cx: &mut Cx, args: Args) -> Result<Value> {
        match self.kind {
            BridgeFunctionKind::Tx => call_tx(cx, args),
            BridgeFunctionKind::Rx => call_rx(cx, args),
            BridgeFunctionKind::Report => call_report(cx, args),
            BridgeFunctionKind::Receipt => call_receipt(cx, args),
            BridgeFunctionKind::Brief => call_brief(cx, args),
        }
    }

    fn browse_args_shape(&self, _cx: &mut Cx) -> Result<Option<sim_kernel::ShapeRef>> {
        let shape: Arc<dyn Shape> = match self.kind {
            BridgeFunctionKind::Tx | BridgeFunctionKind::Report => {
                Arc::new(ListShape::new(vec![Arc::new(AnyShape)]))
            }
            BridgeFunctionKind::Rx => Arc::new(ListShape::new(vec![Arc::new(AnyShape)])),
            BridgeFunctionKind::Receipt => Arc::new(ListShape::new(vec![Arc::new(AnyShape)])),
            BridgeFunctionKind::Brief => Arc::new(ListShape::new(vec![
                Arc::new(AnyShape),
                Arc::new(AnyShape),
                Arc::new(AnyShape),
            ])),
        };
        Ok(Some(shape_value(
            Symbol::qualified(self.symbol().to_string(), "args"),
            shape,
        )))
    }

    fn browse_result_shape(&self, _cx: &mut Cx) -> Result<Option<sim_kernel::ShapeRef>> {
        Ok(Some(shape_value(
            Symbol::qualified(self.symbol().to_string(), "result"),
            Arc::new(AnyShape),
        )))
    }
}

fn call_tx(cx: &mut Cx, args: Args) -> Result<Value> {
    let packet = packet_arg(cx, args, "bridge/tx expects one packet expression")?;
    let request = bridge_tx(cx, &BridgeBook::standard(), &packet)?;
    cx.factory().opaque(Arc::new(request))
}

fn call_rx(cx: &mut Cx, args: Args) -> Result<Value> {
    let response = one_expr_arg(cx, args, "bridge/rx expects one model response expression")?;
    let (packet, report) = crate::bridge_rx(cx, &BridgeBook::standard(), response, None)?;
    cx.factory().expr(sim_kernel::Expr::Map(vec![
        sim_value::build::entry("packet", packet_to_expr(&packet)),
        sim_value::build::entry("report", report.to_expr()),
    ]))
}

fn call_report(cx: &mut Cx, args: Args) -> Result<Value> {
    let packet = packet_arg(cx, args, "bridge/report expects one packet expression")?;
    let report = rx_check(cx, &BridgeBook::standard(), &packet, None)?;
    cx.factory().expr(report.to_expr())
}

fn call_receipt(cx: &mut Cx, args: Args) -> Result<Value> {
    let packet = packet_arg(cx, args, "bridge/receipt expects one packet expression")?;
    let report = rx_check(cx, &BridgeBook::standard(), &packet, None)?;
    let receipt = receipt_packet_for_report(&report, "sim")?;
    cx.factory().expr(packet_to_expr(&receipt))
}

fn call_brief(cx: &mut Cx, args: Args) -> Result<Value> {
    let mut exprs = expr_args(
        cx,
        args,
        "bridge/brief expects target, frame, and return shape",
    )?;
    let [target, frame, return_shape] = take_three(&mut exprs)?;
    let frame = BridgeFramePayload::from_expr(&frame)?;
    let packet = bridge_brief(&target_name(&target)?, frame, return_shape)?;
    cx.factory().expr(packet_to_expr(&packet))
}

fn packet_arg(
    cx: &mut Cx,
    args: Args,
    message: &'static str,
) -> Result<sim_codec_bridge::BridgePacket> {
    expr_to_packet(&one_expr_arg(cx, args, message)?)
}

fn expr_args(cx: &mut Cx, args: Args, message: &'static str) -> Result<Vec<sim_kernel::Expr>> {
    let values = args.into_vec();
    if values.len() != 3 {
        return Err(Error::Eval(message.to_owned()));
    }
    values
        .into_iter()
        .map(|value| value.object().as_expr(cx))
        .collect()
}

fn one_expr_arg(cx: &mut Cx, args: Args, message: &'static str) -> Result<sim_kernel::Expr> {
    let mut values = args.into_vec();
    if values.len() != 1 {
        return Err(Error::Eval(message.to_owned()));
    }
    values.remove(0).object().as_expr(cx)
}

fn take_three(exprs: &mut Vec<sim_kernel::Expr>) -> Result<[sim_kernel::Expr; 3]> {
    let [target, frame, return_shape] =
        std::mem::take(exprs).try_into().map_err(|values: Vec<_>| {
            Error::Eval(format!(
                "bridge/brief expects 3 argument(s), found {}",
                values.len()
            ))
        })?;
    Ok([target, frame, return_shape])
}

fn target_name(expr: &sim_kernel::Expr) -> Result<String> {
    match expr {
        sim_kernel::Expr::String(target) => Ok(target.clone()),
        sim_kernel::Expr::Symbol(target) => Ok(target.as_qualified_str().to_owned()),
        _ => Err(Error::Eval(
            "bridge/brief target must be a string or symbol".to_owned(),
        )),
    }
}
