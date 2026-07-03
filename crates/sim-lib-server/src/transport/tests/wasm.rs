use sim_kernel::{Consistency, EvalRequest, Expr, ReadPolicy, Symbol};
use wat::parse_str as wat_parse_str;

use crate::{FrameKind, ServerAddress, register_wasm_region, server_frame_from_request};

use super::super::{connect_transport_site, encode_transport_frame};
use super::support::{codecs, cx};

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
    .expect("hand-written wasm transport guest should assemble")
}

#[test]
fn wasm_connection_transport_round_trips_through_guest_sim_call() {
    let mut cx = cx();
    let response = crate::ServerFrame::from_expr(
        &mut cx,
        Symbol::qualified("codec", "binary"),
        FrameKind::Response,
        &Expr::String("guest-ok".to_owned()),
        Consistency::RemoteOnly,
        Vec::new(),
        false,
    )
    .unwrap();
    let reply_bytes = encode_transport_frame(&response).unwrap();
    let region = "guest-transport-roundtrip";
    register_wasm_region(region, &guest_wasm_bytes_returning(&reply_bytes)).unwrap();

    let address = ServerAddress::Wasm {
        region: region.to_owned(),
    };
    let (client_site, selected) = connect_transport_site(&mut cx, address, codecs()).unwrap();
    assert_eq!(selected, Symbol::qualified("codec", "binary"));

    let request = server_frame_from_request(
        &mut cx,
        &Symbol::qualified("codec", "binary"),
        EvalRequest {
            expr: Expr::String("request".to_owned()),
            mode: sim_kernel::EvalMode::Eval,
            result_shape: None,
            answer_limit: None,
            stream_buffer: None,
            stream: false,
            required_capabilities: Vec::new(),
            deadline: None,
            consistency: Consistency::RemoteOnly,
            trace: false,
        },
    )
    .unwrap();
    let reply = client_site.answer(&mut cx, request).unwrap();
    assert_eq!(reply.kind, FrameKind::Response);
    assert_eq!(reply.codec, Symbol::qualified("codec", "binary"));
    assert_eq!(
        reply.decode_expr(&mut cx, ReadPolicy::default()).unwrap(),
        Expr::String("guest-ok".to_owned())
    );
}
