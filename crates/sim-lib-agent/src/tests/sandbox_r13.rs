use super::support::{
    as_component, eval_cx, install_agent_lib, install_roundtrip_codecs, request_frame,
};
use sim_kernel::{Error, Expr, ReadPolicy, Symbol};
use sim_lib_server::{EvalSite, eval_reply_from_frame, register_wasm_region};
use std::sync::atomic::{AtomicU64, Ordering};
use wat::parse_str as wat_parse_str;

static NEXT_REGION_ID: AtomicU64 = AtomicU64::new(1);

fn unique_region_name() -> String {
    format!(
        "test-r13-region-{}",
        NEXT_REGION_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn pack_frame_ref(ptr: u32, len: usize) -> u64 {
    ((u32::try_from(len).expect("guest frame length should fit in u32") as u64) << 32) | ptr as u64
}

fn guest_wasm_bytes_returning(reply_bytes: &[u8]) -> Vec<u8> {
    let packed_reply = pack_frame_ref(2048, reply_bytes.len());
    let reply_text = reply_bytes
        .iter()
        .map(|byte| format!("\\{:02x}", byte))
        .collect::<String>();
    wat_parse_str(format!(
        r#"(module
            (memory (export "memory") 1)
            (global $heap (mut i32) (i32.const 4096))
            (data (i32.const 2048) "{reply_text}")
            (func (export "sim_alloc") (param $len i32) (result i32)
                (local $ptr i32)
                global.get $heap
                local.tee $ptr
                local.get $len
                i32.add
                global.set $heap
                local.get $ptr)
            (func (export "sim_manifest") (result i64) i64.const 0)
            (func (export "sim_exports") (result i64) i64.const 0)
            (func (export "sim_call") (param i32 i32 i32 i32) (result i64)
                i64.const {packed_reply})
        )"#
    ))
    .expect("hand-written wasm sandbox guest should assemble")
}

#[test]
fn r13_sandbox_wasm_uses_real_guest_and_requires_configured_region() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let missing = cx
        .call_function(
            &Symbol::qualified("sandbox", "wasm"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":region")).unwrap(),
                cx.factory()
                    .string("missing-r13-region".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    cx.grant_named("sandbox-wasm");
    let missing_request = request_frame(&mut cx, Expr::String("hello".to_owned()));
    let err = as_component(&missing)
        .answer(&mut cx, missing_request)
        .unwrap_err();
    assert!(
        matches!(err, Error::Eval(message) if message == "unknown wasm region missing-r13-region")
    );

    let guest_value = cx.factory().string("guest-ok".to_owned()).unwrap();
    let response = sim_lib_server::server_frame_from_reply(
        &mut cx,
        &Symbol::qualified("codec", "binary"),
        sim_kernel::EvalReply {
            value: guest_value,
            diagnostics: Vec::new(),
            trace: None,
        },
        sim_kernel::Consistency::RemoteOnly,
    )
    .unwrap();
    let reply_bytes = sim_lib_server::encode_transport_frame(&response).unwrap();
    let region = unique_region_name();
    register_wasm_region(&region, &guest_wasm_bytes_returning(&reply_bytes)).unwrap();

    let sandbox = cx
        .call_function(
            &Symbol::qualified("sandbox", "wasm"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":region")).unwrap(),
                cx.factory().string(region).unwrap(),
            ]),
        )
        .unwrap();
    let frame = request_frame(&mut cx, Expr::String("request".to_owned()));
    let reply = as_component(&sandbox).answer(&mut cx, frame).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert_eq!(expr, Expr::String("guest-ok".to_owned()));
}

#[test]
fn r13_sandbox_subprocess_runs_real_child_and_enforces_timeout() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let sandbox = cx
        .call_function(
            &Symbol::qualified("sandbox", "subprocess"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":command")).unwrap(),
                cx.factory()
                    .string(
                        "expr=\"$SIM_SANDBOX_EXPR\"; case \"$expr\" in *+*2*3*) printf '5\\n' ;; *) exit 1 ;; esac"
                            .to_owned(),
                    )
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":max-time")).unwrap(),
                cx.factory().string("1s".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let sum_request = request_frame(
        &mut cx,
        Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::new("+"))),
            args: vec![
                Expr::Number(sim_kernel::NumberLiteral {
                    domain: Symbol::qualified("numbers", "f64"),
                    canonical: "2".to_owned(),
                }),
                Expr::Number(sim_kernel::NumberLiteral {
                    domain: Symbol::qualified("numbers", "f64"),
                    canonical: "3".to_owned(),
                }),
            ],
        },
    );
    let denied = as_component(&sandbox)
        .answer(&mut cx, sum_request)
        .unwrap_err();
    assert!(
        matches!(denied, Error::CapabilityDenied { capability } if capability == sim_kernel::CapabilityName::new("sandbox-subprocess"))
    );

    cx.grant_named("sandbox-subprocess");
    let sum_request = request_frame(
        &mut cx,
        Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::new("+"))),
            args: vec![
                Expr::Number(sim_kernel::NumberLiteral {
                    domain: Symbol::qualified("numbers", "f64"),
                    canonical: "2".to_owned(),
                }),
                Expr::Number(sim_kernel::NumberLiteral {
                    domain: Symbol::qualified("numbers", "f64"),
                    canonical: "3".to_owned(),
                }),
            ],
        },
    );
    let reply = as_component(&sandbox).answer(&mut cx, sum_request).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(matches!(
        expr,
        Expr::Map(entries)
            if entries.iter().any(|(key, value)| *key == Expr::Symbol(Symbol::new("status")) && *value == Expr::String("ok".to_owned()))
                && entries.iter().any(|(key, value)| *key == Expr::Symbol(Symbol::new("stdout")) && *value == Expr::String("5\n".to_owned()))
    ));

    let timed = cx
        .call_function(
            &Symbol::qualified("sandbox", "subprocess"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":command")).unwrap(),
                cx.factory()
                    .string("sleep 1; printf late".to_owned())
                    .unwrap(),
                cx.factory().symbol(Symbol::new(":max-time")).unwrap(),
                cx.factory().string("50ms".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let wait_request = request_frame(&mut cx, Expr::String("wait".to_owned()));
    let err = as_component(&timed)
        .answer(&mut cx, wait_request)
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Eval(message) if message == "sandbox/subprocess timed out after 50ms"
    ));
}

#[test]
fn r13_capability_restricted_intersects_allow_list_and_fails_closed() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("sandbox");
    cx.grant_named("file-write");

    let denied = cx
        .call_function(
            &Symbol::qualified("sandbox", "capability-restricted"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":allow")).unwrap(),
                cx.factory().expr(Expr::Nil).unwrap(),
            ]),
        )
        .unwrap();
    let path = super::support::temp_memory_path("sandbox-r13");
    let memory_expr = Expr::Call {
        operator: Box::new(Expr::Symbol(Symbol::qualified("memory", "file"))),
        args: vec![Expr::String(path.display().to_string())],
    };
    let denied_request = request_frame(&mut cx, memory_expr.clone());
    let err = as_component(&denied)
        .answer(&mut cx, denied_request)
        .unwrap_err();
    assert!(matches!(
        err,
        Error::CapabilityDenied { capability }
            if capability == crate::fs_write_capability()
    ));

    let allowed = cx
        .call_function(
            &Symbol::qualified("sandbox", "capability-restricted"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":allow")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![Expr::Symbol(Symbol::new("file-write"))]))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let allowed_request = request_frame(&mut cx, memory_expr);
    let reply = as_component(&allowed)
        .answer(&mut cx, allowed_request)
        .unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(matches!(
        expr,
        Expr::Map(entries)
            if entries.iter().any(|(key, value)| *key == Expr::Symbol(Symbol::new("kind")) && *value == Expr::Symbol(Symbol::new("episodic")))
    ));
    let _ = std::fs::remove_file(path);
    assert_eq!(ReadPolicy::default(), ReadPolicy::default());
}
