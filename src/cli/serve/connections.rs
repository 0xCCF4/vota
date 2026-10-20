//! The listener's accept loop: every connection is served as HTTP/1 by hyper
//! with the limits of [`Connections`]: at most `max_open` at once (no
//! `accept` beyond it), a header-read timeout (which also ends idle
//! kept-alive connections), and on shutdown a graceful close of every
//! connection, after which those still open at the end of the grace period
//! are dropped.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::ConnectInfo;
use hyper::server::conn::http1;
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinSet;
use tower::ServiceExt as _;

use crate::config::Connections;

/// Serves `app` on `listener` until `shutdown` completes, then closes the
/// connections gracefully within the grace period.
pub(super) async fn serve(
    listener: TcpListener,
    app: Router,
    limits: &Connections,
    shutdown: impl Future<Output = ()>,
) {
    let slots = Arc::new(Semaphore::new(limits.max_open.get() as usize));
    let (close, closing) = watch::channel(false);
    let mut connections = JoinSet::new();
    tokio::pin!(shutdown);
    loop {
        // Finished connections, so the set doesn't grow.
        while connections.try_join_next().is_some() {}
        let slot = tokio::select! {
            () = &mut shutdown => break,
            slot = Arc::clone(&slots).acquire_owned() => slot.expect("the semaphore is never closed"),
        };
        let (stream, peer) = tokio::select! {
            () = &mut shutdown => break,
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    // E.g. out of file descriptors: retry shortly, without
                    // spinning.
                    tracing::warn!(%error, "accepting a connection failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
        };
        let app = app.clone();
        let header_read_timeout = limits.header_read_timeout;
        let mut closing = closing.clone();
        connections.spawn(async move {
            let _slot = slot;
            let service = hyper::service::service_fn(move |mut request| {
                request.extensions_mut().insert(ConnectInfo(peer));
                app.clone().oneshot(request)
            });
            let connection = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(header_read_timeout)
                .serve_connection(TokioIo::new(stream), service);
            tokio::pin!(connection);
            tokio::select! {
                _ = connection.as_mut() => {}
                _ = closing.changed() => {
                    connection.as_mut().graceful_shutdown();
                    let _ = connection.await;
                }
            }
        });
    }
    drop(listener);
    let _ = close.send(true);
    let drained = tokio::time::timeout(limits.shutdown_grace_period, async {
        while connections.join_next().await.is_some() {}
    })
    .await;
    if drained.is_err() {
        tracing::warn!(
            open = connections.len(),
            "connections still open after the shutdown grace period; closing them"
        );
        connections.shutdown().await;
    }
}
