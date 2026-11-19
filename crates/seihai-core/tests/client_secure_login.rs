//! #564 (squirelworks/seihai): a client session with `Login::secure` opens
//! every game server with the Seihai secure login (`SEI00001`), never the
//! build's clear login, the 6.x Login lobby included.

use std::io::Read;
use std::net::TcpListener;
use std::time::Duration;

use seihai_core::client::{run, Login};
use seihai_core::net::SecureLogin;
use seihai_core::protocol::ClientVersion;

fn first_bytes_sent(version: ClientVersion, secure: Option<SecureLogin>) -> Vec<u8> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let server = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        while let Ok(n) = s.read(&mut buf) {
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
            if got.len() > 16 {
                break;
            }
        }
        got
    });
    let login = Login {
        account: "ab".into(),
        password: "cdpass".into(),
        nick: "ab".into(),
        language: "English".into(),
        secure,
    };
    // The fake server hangs up, so the session ends in an error.
    let _ = run(
        &addr,
        version,
        &login,
        None,
        true,
        &mut Vec::new(),
        &mut |_, _| {},
    );
    server.join().unwrap()
}

#[test]
fn a_secure_session_never_sends_the_password_in_clear() {
    let secure = SecureLogin::pinned([7; 32]);
    for v in [
        ClientVersion::V603R7,
        ClientVersion::V512,
        ClientVersion::V222,
    ] {
        let got = first_bytes_sent(v, Some(secure.clone()));
        assert!(
            got.windows(8).any(|w| w == b"SEI00001"),
            "{}: no SEI00001 hello in {:?}",
            v.label(),
            String::from_utf8_lossy(&got)
        );
        assert!(!got.windows(6).any(|w| w == b"cdpass"), "{}", v.label());
    }
}

#[test]
fn without_secure_the_build_logs_in_as_it_always_did() {
    let got = first_bytes_sent(ClientVersion::V603R7, None);
    assert!(!got.windows(8).any(|w| w == b"SEI00001"));
}
