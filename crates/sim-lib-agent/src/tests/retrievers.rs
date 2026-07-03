use super::support::{
    as_component, eval_cx, install_agent_lib, install_roundtrip_codecs, request_frame,
    temp_text_path,
};
use crate::Component;
use sim_kernel::{Error, Expr, Symbol};
use sim_lib_server::{EvalSite, eval_reply_from_frame};
#[cfg(feature = "agent-net")]
use std::{
    io::ErrorKind,
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

#[test]
fn r11_vector_retriever_ranks_closest_document_first() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let retriever = cx
        .call_function(
            &Symbol::qualified("retriever", "vector"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":store")).unwrap(),
                cx.factory().string("ranked-corpus".to_owned()).unwrap(),
                cx.factory().symbol(Symbol::new(":corpus")).unwrap(),
                cx.factory()
                    .expr(Expr::List(vec![
                        Expr::String("rust ownership borrow checker".to_owned()),
                        Expr::String("tropical fruit smoothie banana".to_owned()),
                    ]))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let request = request_frame(
        &mut cx,
        Expr::List(vec![
            Expr::Symbol(Symbol::new("query")),
            Expr::String("borrow checker ownership".to_owned()),
            Expr::Number(sim_kernel::NumberLiteral {
                domain: Symbol::qualified("numbers", "i64"),
                canonical: "1".to_owned(),
            }),
        ]),
    );
    let reply = as_component(&retriever).answer(&mut cx, request).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    let Expr::List(items) = expr else {
        panic!("vector retriever should return a list");
    };
    assert_eq!(items.len(), 1);
    assert!(format!("{:?}", items[0]).contains("borrow checker"));
}

#[test]
fn r11_vector_retriever_reads_shared_store_documents() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let memory = cx
        .call_function(
            &Symbol::qualified("memory", "blackboard"),
            sim_kernel::Args::new(vec![
                cx.factory().string("shared-store".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    cx.call_function(
        &Symbol::qualified("memory", "append"),
        sim_kernel::Args::new(vec![
            memory.clone(),
            cx.factory()
                .expr(Expr::String("database indexing with sim".to_owned()))
                .unwrap(),
        ]),
    )
    .unwrap();

    let retriever = cx
        .call_function(
            &Symbol::qualified("retriever", "vector"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":store")).unwrap(),
                cx.factory().string("shared-store".to_owned()).unwrap(),
            ]),
        )
        .unwrap();
    let request = request_frame(&mut cx, Expr::String("indexing".to_owned()));
    let reply = as_component(&retriever).answer(&mut cx, request).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(format!("{expr:?}").contains("database indexing with sim"));
}

#[test]
fn r11_db_retriever_scans_real_jsonl_store() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("file-read");

    let path = temp_text_path("db");
    std::fs::write(
        &path,
        "{\"id\":\"a\",\"text\":\"alpha beta\"}\n{\"id\":\"b\",\"text\":\"beta gamma\"}\n",
    )
    .unwrap();

    let retriever = cx
        .call_function(
            &Symbol::qualified("retriever", "db"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":path")).unwrap(),
                cx.factory().string(path.display().to_string()).unwrap(),
            ]),
        )
        .unwrap();
    let request = request_frame(
        &mut cx,
        Expr::List(vec![
            Expr::Symbol(Symbol::new("query")),
            Expr::String("beta gamma".to_owned()),
            Expr::Number(sim_kernel::NumberLiteral {
                domain: Symbol::qualified("numbers", "i64"),
                canonical: "2".to_owned(),
            }),
        ]),
    );
    let reply = as_component(&retriever).answer(&mut cx, request).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(format!("{expr:?}").contains("\"b\""));

    let reflected = as_component(&retriever).reflect(&mut cx).unwrap();
    assert!(format!("{reflected:?}").contains(path.to_string_lossy().as_ref()));
    let _ = std::fs::remove_file(path);
}

#[cfg(not(feature = "agent-net"))]
#[test]
fn r11_web_retriever_requires_agent_net_when_network_is_granted() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("network");

    let retriever = cx
        .call_function(
            &Symbol::qualified("retriever", "web"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":endpoint")).unwrap(),
                cx.factory()
                    .string("http://127.0.0.1:1".to_owned())
                    .unwrap(),
            ]),
        )
        .unwrap();
    let request = request_frame(&mut cx, Expr::String("test".to_owned()));
    let err = as_component(&retriever)
        .answer(&mut cx, request)
        .unwrap_err();
    assert!(format!("{err}").contains("agent-net"));
}

#[cfg(feature = "agent-net")]
#[test]
fn r11_web_retriever_fetches_real_http_response() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();
    cx.grant_named("network");

    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let port = listener.local_addr().unwrap().port();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request_text = String::from_utf8_lossy(&request);
        assert!(request_text.starts_with("GET /docs/topic "));
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nContent-Type: text/plain\r\n\r\nhello world",
            )
            .unwrap();
    });

    let retriever = cx
        .call_function(
            &Symbol::qualified("retriever", "web"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":endpoint")).unwrap(),
                cx.factory()
                    .string(format!("http://127.0.0.1:{port}/docs"))
                    .unwrap(),
            ]),
        )
        .unwrap();
    let request = request_frame(&mut cx, Expr::String("topic".to_owned()));
    let reply = as_component(&retriever).answer(&mut cx, request).unwrap();
    let expr = eval_reply_from_frame(&mut cx, &reply)
        .unwrap()
        .value
        .object()
        .as_expr(&mut cx)
        .unwrap();
    assert!(format!("{expr:?}").contains("hello world"));
    handle.join().unwrap();
}

#[cfg(feature = "agent-net")]
fn bind_loopback_listener() -> Option<TcpListener> {
    for _ in 0..3 {
        match TcpListener::bind(("127.0.0.1", 0)) {
            Ok(listener) => return Some(listener),
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("failed to bind loopback listener: {error}"),
        }
    }
    None
}

#[test]
fn r11_web_and_db_reflect_required_capabilities() {
    let mut cx = eval_cx();
    install_roundtrip_codecs(&mut cx);
    install_agent_lib(&mut cx).unwrap();

    let web = cx
        .call_function(
            &Symbol::qualified("retriever", "web"),
            sim_kernel::Args::default(),
        )
        .unwrap();
    let db = cx
        .call_function(
            &Symbol::qualified("retriever", "db"),
            sim_kernel::Args::new(vec![
                cx.factory().symbol(Symbol::new(":path")).unwrap(),
                cx.factory().string("/tmp/test.db".to_owned()).unwrap(),
            ]),
        )
        .unwrap();

    let web_expr = as_component(&web).reflect(&mut cx).unwrap();
    let db_expr = as_component(&db).reflect(&mut cx).unwrap();
    assert!(format!("{web_expr:?}").contains("network"));
    assert!(format!("{db_expr:?}").contains("file-read"));
    let request = request_frame(&mut cx, Expr::String("test".to_owned()));
    let err = as_component(&web).answer(&mut cx, request).unwrap_err();
    assert!(matches!(err, Error::CapabilityDenied { .. }));
}
