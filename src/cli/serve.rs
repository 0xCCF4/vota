mod connections;

use std::sync::Arc;

use anyhow::Context as _;
use tokio::sync::oneshot;

use crate::clock::SystemClock;
use crate::config::Config;
use crate::jobs::purge_sweep;
use crate::{AppState, app};

pub(super) async fn run(config: Config) -> anyhow::Result<()> {
    let addr = config.socket_addr();
    let limits = config.connections.clone();
    let state = AppState::new(config, Arc::new(SystemClock)).await?;

    // Installed before the port is bound, so a stop request that arrives
    // once `listening on` is logged is never lost to the default action.
    let shutdown = shutdown_signal()?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    tracing::info!("listening on {}", listener.local_addr()?);
    let (stop_sweep, sweep_stopped) = oneshot::channel::<()>();
    let sweep = purge_sweep::spawn(state.clone(), async {
        let _ = sweep_stopped.await;
    });
    connections::serve(listener, app(state), &limits, async {
        let signal = shutdown.await;
        tracing::info!("received {signal}, shutting down gracefully");
    })
    .await;
    // The sweep doesn't wait for its next turn; one already running finishes.
    let _ = stop_sweep.send(());
    sweep.await.context("the purge sweep panicked")?;
    tracing::info!("purge sweep stopped; shut down");
    Ok(())
}

/// Installs the handlers now; the future completes on the first SIGTERM
/// (how systemd and most supervisors stop a service) or SIGINT (Ctrl-C),
/// naming it.
#[cfg(unix)]
fn shutdown_signal() -> anyhow::Result<impl Future<Output = &'static str>> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate =
        signal(SignalKind::terminate()).context("installing the SIGTERM handler")?;
    let mut interrupt = signal(SignalKind::interrupt()).context("installing the SIGINT handler")?;
    Ok(async move {
        tokio::select! {
            _ = terminate.recv() => "SIGTERM",
            _ = interrupt.recv() => "SIGINT",
        }
    })
}

/// Completes on Ctrl-C (there is no SIGTERM outside Unix).
#[cfg(not(unix))]
fn shutdown_signal() -> anyhow::Result<impl Future<Output = &'static str>> {
    Ok(async {
        let _ = tokio::signal::ctrl_c().await;
        "Ctrl-C"
    })
}
