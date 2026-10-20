//! The background purge sweep: deletes Ballots past their Purge Date once at
//! startup and then every `purge_interval`, by the injected Clock. Lookups
//! treat such Ballots as gone in between anyway; the sweep removes them (with
//! their Response and Voter Reference) from the database for good.

use std::future::Future;

use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::AppState;
use crate::data::ballot_store;

/// Starts the sweep on the current Tokio runtime. It stops, without waiting
/// for the next sweep, once `shutdown` completes; a sweep already running
/// finishes first. A failed sweep is logged and retried at the next interval.
pub fn spawn(
    state: AppState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        // The first tick completes at once: the startup sweep.
        let mut interval = tokio::time::interval(state.config.purge_interval);
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => break,
                _ = interval.tick() => sweep(&state).await,
            }
        }
    })
}

/// One sweep, logged. Logs only how many Ballots it deleted: never Ballot
/// Tokens or Voter References.
async fn sweep(state: &AppState) {
    match run_once(state).await {
        Ok(0) => tracing::debug!("purge sweep: no Ballots past their Purge Date"),
        Ok(deleted) => tracing::info!(deleted, "purge sweep deleted Ballots past their Purge Date"),
        Err(error) => tracing::warn!(%error, "purge sweep failed"),
    }
}

/// Runs one sweep now, by the app's Clock, as [`spawn`] does at every
/// interval: deletes every Ballot whose Purge Date has come, with its
/// Response and Voter Reference. Returns how many Ballots it deleted.
pub async fn run_once(state: &AppState) -> anyhow::Result<u64> {
    Ok(ballot_store::sweep_purged(&state.db, state.clock.now()).await?)
}
