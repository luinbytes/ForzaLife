use std::{
    net::UdpSocket,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[test]
fn session_returns_the_game_exit_status() {
    let status = Command::new(env!("CARGO_BIN_EXE_forzalife"))
        .args(["session", "/bin/sh", "-c", "exit 7"])
        .env("DISPLAY", ":99999")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("session should run");

    assert_eq!(status.code(), Some(7));
}

#[test]
fn probe_rejects_a_non_fh6_datagram() {
    let port = UdpSocket::bind(("127.0.0.1", 0))
        .expect("temporary UDP socket")
        .local_addr()
        .unwrap()
        .port();
    let mut probe = Command::new(env!("CARGO_BIN_EXE_forzalife"))
        .args(["probe", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("probe should start");

    thread::sleep(Duration::from_millis(100));
    UdpSocket::bind(("127.0.0.1", 0))
        .unwrap()
        .send_to(&[0_u8; 12], ("127.0.0.1", port))
        .expect("send test datagram");

    assert!(!probe.wait().unwrap().success());
}
