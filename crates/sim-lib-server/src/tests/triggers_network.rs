#[cfg(feature = "trigger-imap")]
use std::io::{BufRead, BufReader, Write};
#[cfg(any(feature = "trigger-imap", feature = "trigger-smtp"))]
use std::net::TcpListener;
#[cfg(feature = "trigger-imap")]
use std::sync::mpsc;
use std::{thread, time::Duration};

use super::*;

fn wait_until(timeout_ms: u64, predicate: impl Fn() -> bool) {
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(timeout_ms) {
        if predicate() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for trigger effect");
}

#[cfg(any(feature = "trigger-imap", feature = "trigger-smtp"))]
fn bind_loopback_listener() -> Option<TcpListener> {
    let mut last_error = None;
    for _ in 0..10 {
        match TcpListener::bind(("127.0.0.1", 0)) {
            Ok(listener) => return Some(listener),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                last_error = Some(error);
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => panic!("failed to bind loopback listener: {error}"),
        }
    }
    eprintln!(
        "skipping real network trigger bind in this environment: {}",
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "unknown error".to_owned())
    );
    None
}

fn install_recording_decoder(cx: &mut sim_kernel::Cx) -> Arc<Mutex<Vec<String>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = cx
        .factory()
        .opaque(Arc::new(RecordFn { seen: seen.clone() }))
        .unwrap();
    let decoder = cx.factory().opaque(Arc::new(DecodeRecordFn)).unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "record"), record)
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", "decode-record"), decoder)
        .unwrap();
    seen
}

fn start_server(cx: &mut sim_kernel::Cx, name: &str) {
    let server = cx
        .eval_expr(Expr::Call {
            operator: Box::new(Expr::Symbol(Symbol::qualified("server", "start"))),
            args: vec![],
        })
        .unwrap();
    cx.registry_mut()
        .register_value(Symbol::qualified("test", name), server)
        .unwrap();
}

fn register_trigger(cx: &mut sim_kernel::Cx, server_name: &str, source: Expr) -> sim_kernel::Value {
    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "trigger"))
            .unwrap(),
        vec![
            Expr::Symbol(Symbol::qualified("test", server_name)),
            Expr::Symbol(Symbol::new(":source")),
            quoted(source),
            Expr::Symbol(Symbol::new(":decode")),
            Expr::Symbol(Symbol::qualified("test", "decode-record")),
        ],
    )
    .unwrap()
}

fn stop_server(cx: &mut sim_kernel::Cx, name: &str) {
    cx.call_exprs(
        cx.resolve_function(&Symbol::qualified("server", "stop"))
            .unwrap(),
        vec![Expr::Symbol(Symbol::qualified("test", name))],
    )
    .unwrap();
}

#[cfg(feature = "trigger-smtp")]
fn poll_trigger_until_delivered(
    cx: &mut sim_kernel::Cx,
    handle: &sim_kernel::Value,
    timeout_ms: u64,
) -> Result<u64, sim_kernel::Error> {
    let trigger = handle.object().downcast_ref::<TriggerHandle>().unwrap();
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(timeout_ms) {
        let delivered = trigger.poll(cx)?;
        if delivered > 0 {
            return Ok(delivered);
        }
        thread::sleep(Duration::from_millis(10));
    }
    Ok(0)
}

#[cfg(feature = "trigger-smtp")]
fn stop_trigger_thread(handle: &sim_kernel::Value) {
    handle
        .object()
        .downcast_ref::<TriggerHandle>()
        .unwrap()
        .stop()
        .unwrap();
}

#[test]
fn smtp_loopback_trigger_captures_sent_messages() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant_named("mail-write");
    let seen = install_recording_decoder(&mut cx);
    start_server(&mut cx, "smtp-loopback-server");

    register_trigger(
        &mut cx,
        "smtp-loopback-server",
        Expr::List(vec![
            Expr::Symbol(Symbol::new("smtp")),
            Expr::Symbol(Symbol::new(":address")),
            Expr::String("loopback-smtp".to_owned()),
            Expr::Symbol(Symbol::new(":loopback")),
            Expr::Bool(true),
        ]),
    );

    let source = ServerAddress::Smtp {
        address: "loopback-smtp".to_owned(),
    };
    crate::trigger::enqueue_trigger_event(&source, b"hello smtp".to_vec()).unwrap();

    wait_until(1_000, || {
        seen.lock()
            .expect("smtp loopback seen mutex poisoned")
            .as_slice()
            == ["hello smtp".to_owned()]
    });
    wait_until(1_000, || {
        crate::trigger::loopback_smtp_messages_for(&source)
            .map(|messages| messages == vec![b"hello smtp".to_vec()])
            .unwrap_or(false)
    });

    stop_server(&mut cx, "smtp-loopback-server");
}

#[test]
fn imap_loopback_trigger_uses_queue_double() {
    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant_named("mail-read");
    let seen = install_recording_decoder(&mut cx);
    start_server(&mut cx, "imap-loopback-server");

    register_trigger(
        &mut cx,
        "imap-loopback-server",
        Expr::List(vec![
            Expr::Symbol(Symbol::new("imap")),
            Expr::Symbol(Symbol::new(":address")),
            Expr::String("mail.loopback".to_owned()),
            Expr::Symbol(Symbol::new(":mailbox")),
            Expr::String("INBOX".to_owned()),
            Expr::Symbol(Symbol::new(":loopback")),
            Expr::Bool(true),
        ]),
    );

    crate::trigger::enqueue_trigger_event(
        &ServerAddress::Imap {
            address: "mail.loopback".to_owned(),
            mailbox: "INBOX".to_owned(),
        },
        b"queued mail".to_vec(),
    )
    .unwrap();

    wait_until(1_000, || {
        seen.lock()
            .expect("imap loopback seen mutex poisoned")
            .as_slice()
            == ["queued mail".to_owned()]
    });

    stop_server(&mut cx, "imap-loopback-server");
}

#[cfg(feature = "trigger-smtp")]
#[test]
fn smtp_trigger_delivers_over_real_smtp_session() {
    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let address = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, rx) = mpsc::channel();
    let smtp_thread = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        reader.get_mut().write_all(b"220 local ESMTP\r\n").unwrap();
        reader.get_mut().flush().unwrap();

        let mut data = Vec::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            if line.starts_with("EHLO ") {
                reader.get_mut().write_all(b"250 local\r\n").unwrap();
            } else if line.starts_with("MAIL FROM:") || line.starts_with("RCPT TO:") {
                reader.get_mut().write_all(b"250 ok\r\n").unwrap();
            } else if line == "DATA\r\n" {
                reader.get_mut().write_all(b"354 end data\r\n").unwrap();
                loop {
                    let mut body_line = String::new();
                    reader.read_line(&mut body_line).unwrap();
                    if body_line == ".\r\n" {
                        break;
                    }
                    data.extend_from_slice(body_line.as_bytes());
                }
                reader.get_mut().write_all(b"250 queued\r\n").unwrap();
            } else if line == "QUIT\r\n" {
                reader.get_mut().write_all(b"221 bye\r\n").unwrap();
                reader.get_mut().flush().unwrap();
                break;
            }
            reader.get_mut().flush().unwrap();
        }
        tx.send(data).unwrap();
    });

    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant_named("mail-write");
    let seen = install_recording_decoder(&mut cx);
    start_server(&mut cx, "smtp-real-server");

    let handle = register_trigger(
        &mut cx,
        "smtp-real-server",
        Expr::List(vec![
            Expr::Symbol(Symbol::new("smtp")),
            Expr::Symbol(Symbol::new(":address")),
            Expr::String(address.clone()),
            Expr::Symbol(Symbol::new(":from")),
            Expr::String("from@example.test".to_owned()),
            Expr::Symbol(Symbol::new(":to")),
            Expr::String("to@example.test".to_owned()),
        ]),
    );

    stop_trigger_thread(&handle);
    crate::trigger::enqueue_trigger_event(
        &ServerAddress::Smtp {
            address: address.clone(),
        },
        b"real smtp body".to_vec(),
    )
    .unwrap();

    assert_eq!(
        poll_trigger_until_delivered(&mut cx, &handle, 1_000).unwrap(),
        1
    );
    wait_until(1_000, || {
        seen.lock()
            .expect("smtp real seen mutex poisoned")
            .as_slice()
            == ["real smtp body".to_owned()]
    });
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        b"real smtp body\r\n"
    );
    smtp_thread.join().unwrap();
    stop_server(&mut cx, "smtp-real-server");
}

#[cfg(feature = "trigger-imap")]
#[test]
fn imap_trigger_fetches_unseen_message_and_marks_it_seen() {
    let Some(listener) = bind_loopback_listener() else {
        return;
    };
    let address = format!("127.0.0.1:{}", listener.local_addr().unwrap().port());
    let (tx, rx) = mpsc::channel();
    let imap_thread = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut seen_store = false;
        while std::time::Instant::now() < deadline && !seen_store {
            let (stream, _) = match listener.accept() {
                Ok(pair) => pair,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) => panic!("imap test accept failed: {error}"),
            };
            let mut reader = BufReader::new(stream);
            reader
                .get_mut()
                .write_all(b"* OK IMAP4rev1 ready\r\n")
                .unwrap();
            reader.get_mut().flush().unwrap();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 {
                    break;
                }
                let trimmed = line.trim_end_matches(['\r', '\n']);
                let mut parts = trimmed.splitn(3, ' ');
                let tag = parts.next().unwrap();
                let command = parts.next().unwrap_or_default();
                let rest = parts.next().unwrap_or_default();
                match (command, rest) {
                    ("LOGIN", _) => {
                        write!(reader.get_mut(), "{tag} OK LOGIN completed\r\n").unwrap();
                    }
                    ("SELECT", _) => {
                        write!(
                            reader.get_mut(),
                            "* 1 EXISTS\r\n{tag} OK SELECT completed\r\n"
                        )
                        .unwrap();
                    }
                    ("SEARCH", "UNSEEN") if !seen_store => {
                        write!(
                            reader.get_mut(),
                            "* SEARCH 7\r\n{tag} OK SEARCH completed\r\n"
                        )
                        .unwrap();
                    }
                    ("SEARCH", "UNSEEN") => {
                        write!(
                            reader.get_mut(),
                            "* SEARCH\r\n{tag} OK SEARCH completed\r\n"
                        )
                        .unwrap();
                    }
                    ("FETCH", "7 BODY[]") => {
                        write!(
                            reader.get_mut(),
                            "* 7 FETCH (BODY[] {{10}}\r\nhello imap\r\n)\r\n{tag} OK FETCH completed\r\n"
                        )
                        .unwrap();
                    }
                    ("STORE", "7 +FLAGS (\\Seen)") => {
                        write!(reader.get_mut(), "{tag} OK STORE completed\r\n").unwrap();
                        seen_store = true;
                        tx.send(()).unwrap();
                    }
                    ("LOGOUT", _) => {
                        write!(
                            reader.get_mut(),
                            "* BYE logout\r\n{tag} OK LOGOUT completed\r\n"
                        )
                        .unwrap();
                        reader.get_mut().flush().unwrap();
                        break;
                    }
                    _ => panic!("unexpected imap command: {trimmed}"),
                }
                reader.get_mut().flush().unwrap();
            }
        }
    });

    let mut cx = cx();
    install_server_lib(&mut cx).unwrap();
    cx.grant_named("mail-read");
    let seen = install_recording_decoder(&mut cx);
    start_server(&mut cx, "imap-real-server");

    register_trigger(
        &mut cx,
        "imap-real-server",
        Expr::List(vec![
            Expr::Symbol(Symbol::new("imap")),
            Expr::Symbol(Symbol::new(":address")),
            Expr::String(address),
            Expr::Symbol(Symbol::new(":mailbox")),
            Expr::String("INBOX".to_owned()),
            Expr::Symbol(Symbol::new(":user")),
            Expr::String("alice".to_owned()),
            Expr::Symbol(Symbol::new(":password")),
            Expr::String("secret".to_owned()),
        ]),
    );

    wait_until(1_000, || {
        seen.lock()
            .expect("imap real seen mutex poisoned")
            .as_slice()
            == ["hello imap".to_owned()]
    });
    rx.recv_timeout(Duration::from_secs(2)).unwrap();

    stop_server(&mut cx, "imap-real-server");
    imap_thread.join().unwrap();
}
