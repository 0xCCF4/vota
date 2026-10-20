//! Rate limiting of failed Ballot Token lookups, per client IP; and, with
//! the same machinery and its own table and budget ([`Budget::admin_auth`]),
//! of failed Admin Token checks (`crate::api::admin`).
//!
//! A failed lookup is a uniform not-found (404) from the voting page or the
//! voter API: a malformed, unknown, deleted or purged Ballot Token. Each client
//! may make [`RateLimit::failed_lookups`] of them per [`RateLimit::window`]
//! (a fixed window starting at its first failure). Once the budget is spent,
//! *every* Ballot Token request from that client gets 429 with `Retry-After`
//! until the window ends, including requests for a valid token: a client that
//! is guessing tokens is blocked, not filtered. Successful lookups never spend
//! the budget.
//!
//! The client IP is resolved by [`crate::input::client_ip`] (the TCP peer, or
//! `X-Forwarded-For` behind a trusted proxy; an unreadable header from a
//! trusted proxy gets 400 and is charged to no bucket). An IPv6 client
//! address with an IPv4 address embedded in it (NAT64, SIIT, `nat64_prefixes`)
//! is resolved to that IPv4 address. Other IPv6 clients are counted per /64,
//! since one host usually controls a whole /64. A request without a peer
//! address (the router served without connect info) is not limited; the
//! `serve` command always provides it.
//!
//! Each request reserves one failure in its client's budget before the
//! lookup and gets it back unless the answer is a 404 (also when the request
//! is cancelled), all under one lock: concurrent requests can never make
//! more lookups than the budget. While a client's last failures are in
//! flight, its further requests get 429 until they come back.
//!
//! State is in memory (one instance) and bounded: at most [`MAX_CLIENTS`]
//! clients are tracked. Expired windows are dropped as time passes. When the
//! table is full, a new client takes the place of the oldest client that is
//! not blocked; blocked clients are never evicted, so a blocked client can't
//! lift its own block by failing from many new addresses. If every tracked
//! client is blocked, new clients share one overflow window with the same
//! budget until room frees up. Every step costs O(log n), never a scan of the
//! table. Ballot Token lookups never touch the admin budget, nor failed
//! Admin Token checks the lookup budget.

use std::collections::{BTreeMap, HashMap};
use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde_json::json;

use crate::AppState;
use crate::config::RateLimit;
use crate::input;

/// Upper bound on the number of clients tracked at once.
pub const MAX_CLIENTS: usize = 65_536;

/// Failures per client in the current window: failed Ballot Token lookups,
/// or (a separate table) failed Admin Token checks.
#[derive(Debug, Default)]
pub(crate) struct Failures {
    table: Mutex<Table>,
}

/// How many failures a client may have per window.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Budget {
    failures: u32,
    window: Duration,
}

impl Budget {
    /// Failed Ballot Token lookups: `failed_lookups` per `window`.
    fn lookups(limit: &RateLimit) -> Self {
        Self {
            failures: limit.failed_lookups.get(),
            window: limit.window,
        }
    }

    /// Failed Admin Token checks: `admin_failed_auth` per `admin_window`.
    pub(crate) fn admin_auth(limit: &RateLimit) -> Self {
        Self {
            failures: limit.admin_failed_auth.get(),
            window: limit.admin_window,
        }
    }
}

/// A window's place in the table's order: its start, then the order it was
/// opened in.
type Key = (DateTime<Utc>, u64);

#[derive(Debug, Default)]
struct Table {
    clients: HashMap<IpAddr, Window>,
    /// Every tracked client, oldest window first.
    by_start: BTreeMap<Key, IpAddr>,
    /// The tracked clients that are not blocked, oldest window first: those
    /// that may be evicted to make room.
    unblocked: BTreeMap<Key, IpAddr>,
    /// The window shared by new clients while the table is full of blocked
    /// ones.
    overflow: Option<Window>,
    /// The order number of the next window opened.
    next: u64,
}

#[derive(Debug, Clone, Copy)]
struct Window {
    started: DateTime<Utc>,
    /// Tells this window apart from a later one of the same client.
    order: u64,
    /// Failures, including reserved ones still in flight.
    failures: u32,
}

impl Window {
    fn key(&self) -> Key {
        (self.started, self.order)
    }

    /// `Err(seconds until the window ends)` when its budget is spent.
    fn check(&self, budget: &Budget, now: DateTime<Utc>) -> Result<(), u64> {
        if self.failures < budget.failures {
            return Ok(());
        }
        let end = window_end(self.started, budget);
        // Round up, so a retry at that moment is let through.
        let millis = (end - now).num_milliseconds().max(0) as u64;
        Err(millis.div_ceil(1000).max(1))
    }
}

/// Where a reserved failure was counted.
#[derive(Debug, Clone, Copy)]
enum Slot {
    Client(IpAddr, u64),
    Overflow(u64),
}

impl Table {
    fn open_window(&mut self, now: DateTime<Utc>) -> Window {
        self.next += 1;
        Window {
            started: now,
            order: self.next,
            failures: 0,
        }
    }

    /// Drops every window that has ended by `now`, oldest first.
    fn expire(&mut self, budget: &Budget, now: DateTime<Utc>) {
        while let Some(entry) = self.by_start.first_entry() {
            if now < window_end(entry.key().0, budget) {
                break;
            }
            let (key, ip) = entry.remove_entry();
            self.unblocked.remove(&key);
            self.clients.remove(&ip);
        }
        if self
            .overflow
            .is_some_and(|window| now >= window_end(window.started, budget))
        {
            self.overflow = None;
        }
    }

    fn remove(&mut self, ip: IpAddr) {
        if let Some(window) = self.clients.remove(&ip) {
            self.by_start.remove(&window.key());
            self.unblocked.remove(&window.key());
        }
    }

    /// Reserves one failure for `client`, or says how long it is blocked.
    /// Also returns how many failures its window holds with this one.
    fn reserve(
        &mut self,
        client: IpAddr,
        budget: &Budget,
        now: DateTime<Utc>,
    ) -> Result<(Slot, u32), u64> {
        self.expire(budget, now);
        if let Some(window) = self.clients.get_mut(&client) {
            window.check(budget, now)?;
            window.failures += 1;
            let (key, order, failures) = (window.key(), window.order, window.failures);
            if failures >= budget.failures {
                self.unblocked.remove(&key);
            }
            return Ok((Slot::Client(client, order), failures));
        }
        if self.clients.len() >= MAX_CLIENTS {
            // Room is made only by a client that isn't blocked.
            match self.unblocked.first_key_value() {
                Some((_, &oldest)) => self.remove(oldest),
                None => {
                    let fresh = self.open_window(now);
                    let window = self.overflow.get_or_insert(fresh);
                    window.check(budget, now)?;
                    window.failures += 1;
                    return Ok((Slot::Overflow(window.order), window.failures));
                }
            }
        }
        let mut window = self.open_window(now);
        window.failures = 1;
        self.clients.insert(client, window);
        self.by_start.insert(window.key(), client);
        if window.failures < budget.failures {
            self.unblocked.insert(window.key(), client);
        }
        Ok((Slot::Client(client, window.order), window.failures))
    }

    /// Gives back a reserved failure that turned out not to be one: a window
    /// left without failures is dropped.
    fn refund(&mut self, slot: Slot, budget: &Budget) {
        match slot {
            Slot::Client(ip, order) => {
                let Some(window) = self.clients.get_mut(&ip) else {
                    return;
                };
                if window.order != order {
                    return;
                }
                window.failures = window.failures.saturating_sub(1);
                let window = *window;
                if window.failures == 0 {
                    self.remove(ip);
                } else if window.failures < budget.failures {
                    self.unblocked.insert(window.key(), ip);
                }
            }
            Slot::Overflow(order) => {
                if let Some(window) = self.overflow.as_mut().filter(|w| w.order == order) {
                    window.failures = window.failures.saturating_sub(1);
                    if window.failures == 0 {
                        self.overflow = None;
                    }
                }
            }
        }
    }
}

impl Failures {
    /// Reserves one failure for `client` (an IP, counted in its
    /// [`bucket`]) before the request is answered, or `Err(seconds until
    /// the window ends)` when it is blocked. The reservation is given back
    /// when the returned guard is dropped, unless it is kept.
    pub(crate) fn reserve(
        self: &Arc<Self>,
        client: IpAddr,
        budget: Budget,
        now: DateTime<Utc>,
    ) -> Result<Reservation, u64> {
        let mut table = self.table.lock().unwrap_or_else(|e| e.into_inner());
        let (slot, failures) = table.reserve(bucket(client), &budget, now)?;
        Ok(Reservation {
            failures_table: Arc::clone(self),
            budget,
            slot: Some(slot),
            failures,
        })
    }
}

/// A failure reserved for a request in flight: given back on drop unless
/// [`Reservation::keep`] made it count.
pub(crate) struct Reservation {
    failures_table: Arc<Failures>,
    budget: Budget,
    slot: Option<Slot>,
    /// The client's failures in its window, this one included.
    failures: u32,
}

impl Reservation {
    /// The request failed: the reserved failure counts. Returns the
    /// client's failures in its window, this one included, and whether this
    /// one spent the budget.
    pub(crate) fn keep(mut self) -> (u32, bool) {
        self.slot = None;
        (self.failures, self.failures == self.budget.failures)
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            let mut table = (self.failures_table.table.lock()).unwrap_or_else(|e| e.into_inner());
            table.refund(slot, &self.budget);
        }
    }
}

fn window_end(started: DateTime<Utc>, budget: &Budget) -> DateTime<Utc> {
    let window = chrono::Duration::from_std(budget.window).unwrap_or(chrono::Duration::MAX);
    started
        .checked_add_signed(window)
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

/// The bucket a client IP is counted in: IPv4 addresses individually, IPv6
/// addresses per /64.
fn bucket(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => {
            let prefix = u128::from(v6) & !((1u128 << 64) - 1);
            IpAddr::V6(Ipv6Addr::from(prefix))
        }
    }
}

/// What a surface answers when the rate limit refuses a request.
struct Refusals {
    /// 429: the client spent its budget.
    too_many: fn() -> Response,
    /// 400: a trusted proxy passed on an unreadable `X-Forwarded-For`.
    bad_request: fn() -> Response,
}

/// The JSON body of a 429 (voter and admin APIs).
pub(crate) fn json_too_many() -> Response {
    Json(json!({ "error": "too_many_requests" })).into_response()
}

/// The JSON body of a 400 for an unreadable `X-Forwarded-For` from a
/// trusted proxy (voter and admin APIs).
pub(crate) fn json_bad_forwarded_for() -> Response {
    let body = json!({
        "error": "bad_request",
        "message": "the X-Forwarded-For header could not be read",
    });
    Json(body).into_response()
}

/// Middleware for the voter API.
pub(crate) async fn api(state: State<AppState>, request: Request, next: Next) -> Response {
    let refusals = Refusals {
        too_many: json_too_many,
        bad_request: json_bad_forwarded_for,
    };
    limit(state, request, next, refusals).await
}

/// Middleware for the voting page.
pub(crate) async fn page(state: State<AppState>, request: Request, next: Next) -> Response {
    let refusals = Refusals {
        too_many: || {
            Html(concat!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">",
                "<title>Too many requests</title></head><body>",
                "<h1>Too many requests</h1>",
                "<p>Please try again later.</p></body></html>"
            ))
            .into_response()
        },
        bad_request: || {
            Html(concat!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">",
                "<title>Bad request</title></head><body>",
                "<h1>Bad request</h1>",
                "<p>The request could not be read.</p></body></html>"
            ))
            .into_response()
        },
    };
    limit(state, request, next, refusals).await
}

async fn limit(
    State(state): State<AppState>,
    request: Request,
    next: Next,
    refusals: Refusals,
) -> Response {
    let Some(ConnectInfo(peer)) = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .copied()
    else {
        return next.run(request).await;
    };
    let (trusted, nat64) = (&state.config.trusted_proxies, &state.config.nat64_prefixes);
    let Ok(client) = input::client_ip(peer.ip(), request.headers(), trusted, nat64) else {
        // Charged to no bucket: not the proxy's, which everyone it passes on
        // without a usable header would share, and not any address in it.
        return refuse((refusals.bad_request)(), StatusCode::BAD_REQUEST, None);
    };
    let budget = Budget::lookups(&state.config.rate_limit);
    let reservation = match state
        .failed_lookups
        .reserve(client, budget, state.clock.now())
    {
        Ok(reservation) => reservation,
        Err(retry_after) => {
            let status = StatusCode::TOO_MANY_REQUESTS;
            return refuse((refusals.too_many)(), status, Some(retry_after));
        }
    };
    let response = next.run(request).await;
    if response.status() == StatusCode::NOT_FOUND {
        let _ = reservation.keep();
    }
    response
}

/// `response` with this status, the error headers and, for a 429, when to
/// retry (in seconds).
pub(crate) fn refuse(response: Response, status: StatusCode, retry_after: Option<u64>) -> Response {
    let mut response = super::error_page::harden(response);
    *response.status_mut() = status;
    if let Some(retry_after) = retry_after {
        response
            .headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(retry_after));
    }
    response
}
