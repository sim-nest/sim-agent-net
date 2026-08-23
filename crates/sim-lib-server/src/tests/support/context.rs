use std::sync::atomic::AtomicU64;
pub(crate) use std::{
    fs,
    sync::atomic::Ordering,
    sync::{Arc, Mutex},
};

use sim_codec_algol::AlgolCodecLib;
use sim_codec_binary::BinaryCodecLib;
use sim_codec_json::JsonCodecLib;
use sim_codec_lisp::LispCodecLib;
use sim_kernel::{DefaultFactory, EagerPolicy, Expr, QuoteMode, StrictNames, Symbol};
use wat::parse_str as wat_parse_str;

pub(crate) use sim_kernel::{
    EvalFabric, EvalRequest, ReadPolicy, eval_fabric_capability, eval_remote_capability,
    read_eval_capability,
};

pub(crate) use crate::wasm::lookup_wasm_region;

pub(crate) static NEXT_TEST_VALUE_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) fn cx() -> sim_kernel::Cx {
    let mut cx = sim_kernel::Cx::new(
        Arc::new(EagerPolicy),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    );
    install_codecs(&mut cx);
    cx
}

pub(crate) fn strict_name_cx() -> sim_kernel::Cx {
    let mut cx = sim_kernel::Cx::new(
        Arc::new(StrictNames(EagerPolicy)),
        Arc::new(DefaultFactory),
        sim_kernel::HandleSeed::new(1),
    );
    install_codecs(&mut cx);
    cx
}

fn install_codecs(cx: &mut sim_kernel::Cx) {
    let lisp = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
    let json = JsonCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&json).unwrap();
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
    let algol = AlgolCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&algol).unwrap();
}

pub(crate) fn installed_codecs() -> Vec<Symbol> {
    vec![
        Symbol::qualified("codec", "lisp"),
        Symbol::qualified("codec", "json"),
        Symbol::qualified("codec", "binary"),
        Symbol::qualified("codec", "algol"),
    ]
}

pub(crate) fn quoted(expr: Expr) -> Expr {
    Expr::Quote {
        mode: QuoteMode::Quote,
        expr: Box::new(expr),
    }
}

pub(crate) fn minimal_wasm_guest_bytes() -> Vec<u8> {
    wat_parse_str(
        r#"(module
            (memory (export "memory") 1)
            (func (export "sim_alloc") (param i32) (result i32) i32.const 0)
            (func (export "sim_manifest") (result i64) i64.const 0)
            (func (export "sim_exports") (result i64) i64.const 0)
            (func (export "sim_call") (param i32 i32 i32 i32) (result i64) i64.const 0)
        )"#,
    )
    .unwrap()
}
