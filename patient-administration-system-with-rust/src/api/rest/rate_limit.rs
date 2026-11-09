//! Request limits per client, and a lockout after repeated `401`s, with `axa-request-rate-limit`.
//!
//! - **The limit:** `PAS_RATE_LIMIT_RPM` requests per client address in each fixed minute (`0` leaves it
//!   out). A client can send up to twice that across the end of a minute, and there is no separate burst.
//! - **The lockout:** a client that earns `LOCKOUT_CLIENT_ADDRESS_AFTER_COUNT` `401`s in
//!   [`LOCKOUT_WINDOW_SECONDS`] is refused until that window ends. It is off unless that is set above `0`:
//!   the UI servers call the API for all their users from one address, so a lockout there would refuse
//!   every user because some had expired tokens. Turn it on where callers reach the API directly. A client
//!   that signs in correctly is never slowed. It sits outside authentication, so it sees its `401`s.
//!
//! Both answer `429` as a problem with `code` `too_many_requests`, `Retry-After`, and the `RateLimit-*`
//! headers. `/api/health` is never counted by the limit.
//!
//! **The client** is the peer address of the connection, which [`client_address`] copies from
//! `ConnectInfo` to where `axa-request-rate-limit` reads it. `X-Forwarded-For` is never read, because a caller
//! writes it. Behind a proxy every request comes from the proxy, and all clients share one limit. A
//! request with no address (a test driving the router directly) shares one limit with every other such
//! request. An address is held only as a keyed hash, and IPv6 counts by its `/64`.
//!
//! Each process counts for itself, so several replicas give a client one limit each. A limit protects the
//! service from a noisy client; it does not stop a distributed attack.

use std::net::{IpAddr, SocketAddr};
use std::num::NonZeroU32;

use axa_request_rate_limit::{Limiter, by_key, client_key, lock_out_failures};
use axum::Router;
use axum::extract::{ConnectInfo, Request};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::Response;

/// The paths the limit never counts.
pub const EXEMPT: [&str; 1] = ["/api/health"];

/// The window the lockout counts `401`s in, and holds a locked-out client for: 15 minutes.
pub const LOCKOUT_WINDOW_SECONDS: u32 = 900;

/// The limits the server runs with. `None` leaves that layer out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimits {
    /// Requests per client in each minute.
    pub requests_per_minute: Option<NonZeroU32>,
    /// `401`s per client in [`LOCKOUT_WINDOW_SECONDS`] before it is locked out.
    pub auth_failures: Option<NonZeroU32>,
}

impl RateLimits {
    /// From the settings, where `0` turns a layer off.
    pub fn new(requests_per_minute: u32, auth_failures: u32) -> Self {
        Self {
            requests_per_minute: NonZeroU32::new(requests_per_minute),
            auth_failures: NonZeroU32::new(auth_failures),
        }
    }
}

/// Put the connection's peer address where `axa-request-rate-limit` reads it: a plain [`IpAddr`] request extension,
/// as `axa`'s own stack resolves it. A request with no `ConnectInfo` is left without one.
pub async fn client_address(mut request: Request, next: Next) -> Response {
    if let Some(ConnectInfo(peer)) = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .copied()
    {
        request.extensions_mut().insert::<IpAddr>(peer.ip());
    }
    next.run(request).await
}

/// `app` with the lockout and the limit. Call it after authentication is added, so that both sit outside it.
pub fn with_rate_limits(mut app: Router, limits: RateLimits) -> Router {
    if let Some(failures) = limits.auth_failures {
        let window = NonZeroU32::new(LOCKOUT_WINDOW_SECONDS).unwrap_or(NonZeroU32::MIN);
        app = app.layer(from_fn_with_state(
            Limiter::new(failures, window),
            lock_out_failures,
        ));
    }
    if let Some(per_minute) = limits.requests_per_minute {
        let window = NonZeroU32::new(60).unwrap_or(NonZeroU32::MIN);
        // `by_key` rather than `by_client`, so that `/api/health` is not counted. The key is `Some` for every
        // other request, with or without an address, so a request with none shares one limit, not none.
        let limiter = Limiter::new(per_minute, window).key(|request: &Request| {
            (!EXEMPT.contains(&request.uri().path())).then(|| client_key(request.extensions()))
        });
        app = app.layer(from_fn_with_state(limiter, by_key));
    }
    // Outermost, so that both limits see the address.
    app.layer(from_fn(client_address))
}
