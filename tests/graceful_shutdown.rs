//! `serve` as a real process: it shuts down gracefully on SIGTERM (what
//! systemd and most supervisors send) as well as on SIGINT (Ctrl-C): it logs
//! the signal, stops the purge sweep and exits with status 0.
#![cfg(all(unix, feature = "server"))]

mod common;

use common::server::Server;

#[test]
fn serve_shuts_down_gracefully_on_sigterm() {
    let server = Server::start();

    server.signal("TERM");

    let (code, log) = server.wait_for_exit();
    assert_eq!(code, Some(0), "{log}");
    assert!(
        log.contains("received SIGTERM, shutting down gracefully"),
        "{log}"
    );
    assert!(log.contains("purge sweep stopped"), "{log}");
}

#[test]
fn serve_still_shuts_down_gracefully_on_sigint() {
    let server = Server::start();

    server.signal("INT");

    let (code, log) = server.wait_for_exit();
    assert_eq!(code, Some(0), "{log}");
    assert!(
        log.contains("received SIGINT, shutting down gracefully"),
        "{log}"
    );
    assert!(log.contains("purge sweep stopped"), "{log}");
}
