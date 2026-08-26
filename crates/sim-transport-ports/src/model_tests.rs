use super::*;
#[test]
fn deterministic_fault_profiles_are_exact() {
    let mut s = ModelStream::new(Profile {
        fragment: Some(2),
        lose_writes: vec![1],
        capacity: Some(3),
        peer_half_close_after: Some(1),
        ..Profile::default()
    });
    assert_eq!(s.write(b"abcd").unwrap(), 2);
    assert_eq!(s.write(b"xx").unwrap(), 2);
    let mut out = [0; 4];
    assert_eq!(s.read(&mut out).unwrap(), 2);
    assert_eq!(&out[..2], b"ab");
    assert_eq!(s.read(&mut out).unwrap(), 0);
}
#[test]
fn half_close_and_backpressure_are_visible() {
    let mut s = ModelStream::new(Profile {
        capacity: Some(1),
        ..Profile::default()
    });
    s.write_all(b"a").unwrap();
    assert_eq!(s.write(b"b").unwrap_err().kind(), io::ErrorKind::WouldBlock);
    s.shutdown(Half::Write).unwrap();
    assert_eq!(s.write(b"c").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
}
#[test]
fn domain_failures_and_optional_ipc_are_explicit() {
    let address = SocketAddress::Ip {
        address: "127.0.0.1".parse().unwrap(),
        port: 7,
    };
    let ports = ModelPorts::new(Profile {
        dns_failure: true,
        address_in_use: true,
        cancel_after: Some(0),
        ..Profile::default()
    });
    assert_eq!(
        ports.resolve("invalid", 7).unwrap_err().kind,
        TransportErrorKind::DnsFailure
    );
    assert_eq!(
        ports.listen_tcp(&address).err().unwrap().kind,
        TransportErrorKind::AddressInUse
    );
    assert_eq!(
        ports.connect_tcp(&address).err().unwrap().kind,
        TransportErrorKind::Cancelled
    );
    assert_eq!(
        ports
            .connect(&IpcAddress::WindowsPipe("sim".into()))
            .err()
            .unwrap()
            .kind,
        TransportErrorKind::Unsupported
    );
}
