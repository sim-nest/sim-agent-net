use std::sync::Arc;

use sim_codec_binary::BinaryCodecLib;
use std::time::Duration;

use sim_kernel::{
    CapabilityName, Consistency, Cx, DefaultFactory, EagerPolicy, EvalMode, EvalRequest, Expr,
    ObjectEncoding, StrictNames, Symbol, capability::eval_remote_capability,
    read_construct_capability,
};
use sim_lib_server::{EvalSite, LocalEvalSite, ServerAddress, server_frame_from_request};
use sim_table_db::install_db_dir_lib;

use crate::{
    RemoteDirDescriptor, remote_dir_class_symbol, remote_dir_value, table_remote_capability,
    wrap_remote_table_site,
};

fn cx() -> Cx {
    let mut cx = Cx::new(Arc::new(EagerPolicy), Arc::new(DefaultFactory));
    install_codecs(&mut cx);
    cx
}

fn strict_name_cx() -> Cx {
    let mut cx = Cx::new(Arc::new(StrictNames(EagerPolicy)), Arc::new(DefaultFactory));
    install_codecs(&mut cx);
    cx
}

fn install_codecs(cx: &mut Cx) {
    let binary = BinaryCodecLib::new(cx.registry_mut().fresh_codec_id());
    cx.load_lib(&binary).unwrap();
}

fn grant(cx: &mut Cx, capabilities: &[sim_kernel::CapabilityName]) {
    for capability in capabilities {
        cx.grant(capability.clone());
    }
}

fn table_db_capability() -> CapabilityName {
    CapabilityName::new("table.db")
}

fn table_db_read_capability() -> CapabilityName {
    CapabilityName::new("table.db.read")
}

fn table_db_write_capability() -> CapabilityName {
    CapabilityName::new("table.db.write")
}

fn table_db_mkdir_capability() -> CapabilityName {
    CapabilityName::new("table.db.mkdir")
}

fn table_db_rmdir_capability() -> CapabilityName {
    CapabilityName::new("table.db.rmdir")
}

fn remote_site(cx: &mut Cx) -> Arc<dyn EvalSite> {
    grant(
        cx,
        &[
            table_db_capability(),
            table_db_read_capability(),
            table_db_write_capability(),
            table_db_mkdir_capability(),
            table_db_rmdir_capability(),
        ],
    );
    let root = install_db_dir_lib(cx).unwrap();
    let inner: Arc<dyn EvalSite> = Arc::new(LocalEvalSite::new(
        ServerAddress::Local,
        vec![Symbol::qualified("codec", "binary")],
    ));
    wrap_remote_table_site(inner, root)
}

#[test]
fn remote_dir_roundtrips_against_in_process_site() {
    let mut cx = cx();
    let site = remote_site(&mut cx);
    grant(
        &mut cx,
        &[table_remote_capability(), eval_remote_capability()],
    );
    let remote = remote_dir_value(&mut cx, site, Symbol::qualified("codec", "binary")).unwrap();
    let table = remote.object().as_table_impl().unwrap();
    let dir = remote.object().as_dir().unwrap();
    let root_value = cx.factory().string("root".to_owned()).unwrap();

    table.set(&mut cx, Symbol::new("x"), root_value).unwrap();
    assert_eq!(
        table
            .get(&mut cx, Symbol::new("x"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::String("root".to_owned())
    );
    assert_eq!(table.keys(&mut cx).unwrap(), vec![Symbol::new("x")]);
    assert_eq!(table.len(&mut cx).unwrap(), 1);

    let replacement = cx.factory().string("next").unwrap();
    let cas = table
        .compare_exchange(
            &mut cx,
            Symbol::new("x"),
            sim_kernel::TableExpected::Value(Expr::String("root".to_owned())),
            sim_kernel::TableReplacement::Value(replacement),
        )
        .unwrap();
    assert!(cas.exchanged);
    assert_eq!(
        cas.observed,
        sim_kernel::TableObserved::Value(Expr::String("root".to_owned()))
    );

    let sub = dir.mkdir(&mut cx, Symbol::new("sub")).unwrap();
    let sub_dir = sub.object().as_dir().unwrap();
    assert!(dir.is_dir(&mut cx, Symbol::new("sub")).unwrap());
    assert!(dir.opendir(&mut cx, Symbol::new("sub")).unwrap().is_some());
    let child_value = cx.factory().string("child".to_owned()).unwrap();
    sub_dir
        .as_table_impl()
        .unwrap()
        .set(&mut cx, Symbol::new("y"), child_value)
        .unwrap();

    let opened = dir.opendir(&mut cx, Symbol::new("sub")).unwrap().unwrap();
    assert_eq!(
        opened
            .object()
            .as_table_impl()
            .unwrap()
            .get(&mut cx, Symbol::new("y"))
            .unwrap()
            .object()
            .as_expr(&mut cx)
            .unwrap(),
        Expr::String("child".to_owned())
    );
}

fn table_request(op: &str, args: Vec<Expr>) -> EvalRequest {
    let mut call_args = vec![Expr::List(Vec::new())];
    call_args.extend(args);
    EvalRequest {
        expr: Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("table", op))),
            args: call_args,
        },
        mode: EvalMode::Eval,
        result_shape: None,
        answer_limit: None,
        stream_buffer: None,
        stream: false,
        required_capabilities: Vec::new(),
        deadline: Some(Duration::from_secs(5)),
        consistency: Consistency::RemoteOnly,
        trace: false,
    }
}

#[test]
fn server_rejects_malformed_and_unknown_table_ops() {
    let mut cx = strict_name_cx();
    let site = remote_site(&mut cx);
    let codec = Symbol::qualified("codec", "binary");

    // A known op (`get`) with no key argument fails closed with the original
    // per-op arity message rather than silently succeeding.
    let frame =
        server_frame_from_request(&mut cx, &codec, table_request("get", Vec::new())).unwrap();
    let err = site.answer(&mut cx, frame).unwrap_err();
    assert!(err.to_string().contains("table/get expects path and key"));

    // A `del` with a non-symbol key fails closed as a type mismatch.
    let frame = server_frame_from_request(
        &mut cx,
        &codec,
        table_request("del", vec![Expr::String("x".to_owned())]),
    )
    .unwrap();
    assert!(site.answer(&mut cx, frame).is_err());

    // An unknown `table/<op>` is not handled by the table site; it falls
    // through to the inner site, which has no such callable and errors.
    let frame =
        server_frame_from_request(&mut cx, &codec, table_request("bogus", Vec::new())).unwrap();
    assert!(site.answer(&mut cx, frame).is_err());
}

#[test]
fn remote_dir_requires_table_remote_for_construction_and_eval_remote_for_ops() {
    let mut cx = cx();
    let site = remote_site(&mut cx);

    let err =
        remote_dir_value(&mut cx, site.clone(), Symbol::qualified("codec", "binary")).unwrap_err();
    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability } if capability == table_remote_capability()
    ));

    cx.grant(table_remote_capability());
    let remote = remote_dir_value(&mut cx, site, Symbol::qualified("codec", "binary")).unwrap();
    let table = remote.object().as_table_impl().unwrap();
    let err = table.get(&mut cx, Symbol::new("x")).unwrap_err();
    assert!(matches!(
        err,
        sim_kernel::Error::CapabilityDenied { capability } if capability == eval_remote_capability()
    ));
}

#[test]
fn remote_dir_all_ops_require_eval_remote() {
    let mut cx = cx();
    let site = remote_site(&mut cx);
    cx.grant(table_remote_capability());

    let remote = remote_dir_value(&mut cx, site, Symbol::qualified("codec", "binary")).unwrap();
    let table = remote.object().as_table_impl().unwrap();
    let dir = remote.object().as_dir().unwrap();
    let value = cx.factory().string("value".to_owned()).unwrap();

    assert!(matches!(
        table.set(&mut cx, Symbol::new("x"), value),
        Err(sim_kernel::Error::CapabilityDenied { capability })
            if capability == eval_remote_capability()
    ));
    assert!(matches!(
        table.keys(&mut cx),
        Err(sim_kernel::Error::CapabilityDenied { capability })
            if capability == eval_remote_capability()
    ));
    assert!(matches!(
        table.len(&mut cx),
        Err(sim_kernel::Error::CapabilityDenied { capability })
            if capability == eval_remote_capability()
    ));
    assert!(matches!(
        dir.mkdir(&mut cx, Symbol::new("sub")),
        Err(sim_kernel::Error::CapabilityDenied { capability })
            if capability == eval_remote_capability()
    ));
    assert!(matches!(
        dir.rmdir(&mut cx, Symbol::new("sub")),
        Err(sim_kernel::Error::CapabilityDenied { capability })
            if capability == eval_remote_capability()
    ));
}

#[test]
fn remote_dir_citizen_round_trips_as_descriptor_only() {
    let mut cx = cx();
    let site = remote_site(&mut cx);
    cx.load_lib(&sim_citizen::CitizenLib::all()).unwrap();
    grant(
        &mut cx,
        &[table_remote_capability(), eval_remote_capability()],
    );
    cx.grant(read_construct_capability());
    let remote = remote_dir_value(&mut cx, site, Symbol::qualified("codec", "binary")).unwrap();
    let child = remote
        .object()
        .as_dir()
        .unwrap()
        .mkdir(&mut cx, Symbol::new("child"))
        .unwrap();

    sim_citizen::check_value_fixture(&mut cx, child.clone()).unwrap();

    let ObjectEncoding::Constructor { args, .. } = child
        .object()
        .as_object_encoder()
        .unwrap()
        .object_encoding(&mut cx)
        .unwrap()
    else {
        panic!("expected constructor encoding");
    };
    let args = args
        .iter()
        .map(|arg| sim_citizen::value_from_expr(&mut cx, arg))
        .collect::<sim_kernel::Result<Vec<_>>>()
        .unwrap();
    let decoded = cx.read_construct(&remote_dir_class_symbol(), args).unwrap();
    let descriptor = decoded
        .object()
        .as_any()
        .downcast_ref::<RemoteDirDescriptor>()
        .expect("expected remote descriptor");

    assert_eq!(descriptor.site_kind, "remote-table");
    assert_eq!(descriptor.codec, Symbol::qualified("codec", "binary"));
    assert_eq!(descriptor.path, vec!["child".to_owned()]);
    assert!(decoded.object().as_table_impl().is_none());
    assert!(decoded.object().as_dir().is_none());
}

#[test]
fn remote_dir_citizen_rejects_malformed_path() {
    let mut cx = cx();
    cx.load_lib(&sim_citizen::CitizenLib::all()).unwrap();
    cx.grant(read_construct_capability());
    let args = [
        Expr::Symbol(Symbol::new("v0")),
        Expr::String("local".to_owned()),
        Expr::Symbol(Symbol::qualified("codec", "binary")),
        Expr::List(vec![Expr::String("a/b".to_owned())]),
    ]
    .iter()
    .map(|arg| sim_citizen::value_from_expr(&mut cx, arg))
    .collect::<sim_kernel::Result<Vec<_>>>()
    .unwrap();

    let err = cx
        .read_construct(&remote_dir_class_symbol(), args)
        .unwrap_err();
    assert!(err.to_string().contains("illegal segment"));
}
