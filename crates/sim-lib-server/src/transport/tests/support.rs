use std::sync::Arc;

use sim_codec_binary::BinaryCodecLib;
use sim_codec_json::JsonCodecLib;
use sim_codec_lisp::LispCodecLib;
use sim_kernel::{Cx, DefaultFactory, EagerPolicy, Symbol};

#[cfg(unix)]
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    let lisp = LispCodecLib::new(cx.registry_mut().fresh_codec_id()).unwrap();
    cx.load_lib(&lisp).unwrap();
    let json = JsonCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&json).unwrap();
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
    cx
}

pub(crate) fn codecs() -> Vec<Symbol> {
    vec![
        Symbol::qualified("codec", "binary"),
        Symbol::qualified("codec", "lisp"),
        Symbol::qualified("codec", "json"),
    ]
}

#[cfg(unix)]
pub(crate) fn unix_socket_path(name: &str) -> PathBuf {
    static NEXT_UNIX_SOCKET_ID: AtomicU64 = AtomicU64::new(1);
    let unique = NEXT_UNIX_SOCKET_ID.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("sim-say-{name}-{nanos}-{unique}.sock"))
}
