//! The listener's connection limits, on a real socket of the real binary:
//! a client that doesn't send its request headers in time (nothing, half a
//! request, or nothing more on a kept-alive connection) is disconnected; at
//! most `max_open` connections are served at once; and a shutdown waits for
//! open connections only up to its grace period.
#![cfg(all(unix, feature = "server"))]

mod common;

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use common::server::Server;

fn server(variables: &[(&str, &str)]) -> Server {
    Server::start_with(|command, _| {
        for (name, value) in variables {
            command.env(name, value);
        }
    })
}

fn connect(server: &Server) -> TcpStream {
    let stream = TcpStream::connect(server.addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    stream
}

const HEALTH: &[u8] = b"GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n";

/// Reads until the server closes the connection; how long that took and
/// what it sent.
fn until_closed(stream: &mut TcpStream) -> (Duration, String) {
    let start = Instant::now();
    let mut received = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => received.extend_from_slice(&buffer[..n]),
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(error) => panic!("still open after {:?}: {error}", start.elapsed()),
        }
    }
    (
        start.elapsed(),
        String::from_utf8_lossy(&received).into_owned(),
    )
}

/// Reads one response to a `/healthz` request (it ends in `}`).
fn response(stream: &mut TcpStream) -> String {
    let mut received = Vec::new();
    let mut buffer = [0; 4096];
    while !received.ends_with(b"}") {
        let n = stream.read(&mut buffer).unwrap();
        assert!(n > 0, "closed: {}", String::from_utf8_lossy(&received));
        received.extend_from_slice(&buffer[..n]);
    }
    String::from_utf8(received).unwrap()
}

#[test]
fn clients_that_dont_send_their_headers_in_time_are_disconnected() {
    let server = server(&[("VOTA_CONNECTIONS__HEADER_READ_TIMEOUT", "1s")]);

    let mut silent = connect(&server);
    let mut half_sent = connect(&server);
    half_sent
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: loc")
        .unwrap();
    let mut kept_alive = connect(&server);
    kept_alive.write_all(HEALTH).unwrap();
    assert!(response(&mut kept_alive).starts_with("HTTP/1.1 200 OK"));

    for (what, stream) in [
        ("sending nothing", &mut silent),
        ("half a request", &mut half_sent),
        ("idle after a response", &mut kept_alive),
    ] {
        let (took, received) = until_closed(stream);
        assert!(
            took < Duration::from_secs(5),
            "{what}: closed after {took:?}"
        );
        assert!(!received.contains("200 OK"), "{what}: {received}");
    }
}

#[test]
fn at_most_max_open_connections_are_served_at_once() {
    let server = server(&[
        ("VOTA_CONNECTIONS__MAX_OPEN", "2"),
        ("VOTA_CONNECTIONS__HEADER_READ_TIMEOUT", "60s"),
    ]);
    let mut first = connect(&server);
    let second = connect(&server);
    first.write_all(HEALTH).unwrap();
    assert!(response(&mut first).starts_with("HTTP/1.1 200 OK"));

    // A third connection waits in the listen queue, unanswered...
    let mut third = connect(&server);
    third
        .set_read_timeout(Some(Duration::from_millis(1500)))
        .unwrap();
    third.write_all(HEALTH).unwrap();
    let mut buffer = [0; 64];
    let waiting = third.read(&mut buffer);
    assert!(waiting.is_err(), "answered beyond the cap: {waiting:?}");

    // ...until another one closes.
    drop(second);
    third
        .set_read_timeout(Some(Duration::from_secs(20)))
        .unwrap();
    assert!(response(&mut third).starts_with("HTTP/1.1 200 OK"));
}

#[test]
fn a_half_sent_request_delays_shutdown_by_at_most_the_grace_period() {
    let server = server(&[
        ("VOTA_CONNECTIONS__HEADER_READ_TIMEOUT", "60s"),
        ("VOTA_CONNECTIONS__SHUTDOWN_GRACE_PERIOD", "1s"),
    ]);
    let mut half_sent = connect(&server);
    half_sent
        .write_all(b"GET /healthz HTTP/1.1\r\nHost: loc")
        .unwrap();
    // The server has the connection before it is told to stop.
    let mut other = connect(&server);
    other.write_all(HEALTH).unwrap();
    assert!(response(&mut other).starts_with("HTTP/1.1 200 OK"));

    server.signal("TERM");

    let (code, log, took) = server.wait_for_exit_within(Duration::from_secs(20));
    assert_eq!(code, Some(0), "{log}");
    assert!(took >= Duration::from_millis(900), "exited after {took:?}");
    assert!(took < Duration::from_secs(5), "exited after {took:?}");
    assert!(log.contains("shutdown grace period"), "{log}");
    assert!(log.contains("purge sweep stopped"), "{log}");
    let (_, received) = until_closed(&mut half_sent);
    assert_eq!(received, "");
}

#[test]
fn without_unfinished_requests_shutdown_does_not_wait_for_the_grace_period() {
    let server = server(&[("VOTA_CONNECTIONS__SHUTDOWN_GRACE_PERIOD", "30s")]);
    let mut kept_alive = connect(&server);
    kept_alive.write_all(HEALTH).unwrap();
    assert!(response(&mut kept_alive).starts_with("HTTP/1.1 200 OK"));

    server.signal("TERM");

    let (code, log, took) = server.wait_for_exit_within(Duration::from_secs(20));
    assert_eq!(code, Some(0), "{log}");
    assert!(took < Duration::from_secs(5), "exited after {took:?}");
    assert!(!log.contains("shutdown grace period"), "{log}");
}
