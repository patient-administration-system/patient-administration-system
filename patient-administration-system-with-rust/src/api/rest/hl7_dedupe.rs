//! The HL7 v2 routes answer a resent message from its first ACK instead of processing it again (NFR-31,
//! HUB-2). This layer reads the message's MSH and runs the handler through [`crate::hl7v2::dedupe::guarded`].
//! `parse` changes nothing and `batch` is guarded per message inside it, so both pass straight through.

use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{StatusCode, header::CONTENT_TYPE},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::api::rest::AppState;
use crate::hl7v2::ack::Inbound;
use crate::hl7v2::parse_message;

/// The largest HL7 v2 body this layer reads, the same as the MLLP frame limit.
const MAX_BODY: usize = 1024 * 1024;

pub async fn hl7_dedupe_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    // Anything posted here arrived, whether or not it parses (the silence signal, NFR-34).
    crate::metrics::hl7_received();
    let path = req.uri().path();
    if path.ends_with("/parse") || path.ends_with("/batch") {
        return next.run(req).await;
    }
    let (parts, body) = req.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        return (StatusCode::PAYLOAD_TOO_LARGE, "HL7 v2 message too large").into_response();
    };
    // A body that will not parse goes to the handler, which answers AR; it cannot be recognised anyway.
    let inbound = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| parse_message(text).ok())
        .map(|m| Inbound::of(&m));
    let Some(inbound) = inbound else {
        return next
            .run(Request::from_parts(parts, Body::from(bytes)))
            .await;
    };
    let process = async move {
        let resp = next
            .run(Request::from_parts(parts, Body::from(bytes)))
            .await;
        let body = to_bytes(resp.into_body(), 4 * MAX_BODY)
            .await
            .unwrap_or_default();
        String::from_utf8(body.to_vec()).unwrap_or_default()
    };
    let ack = crate::hl7v2::dedupe::guarded(&state.db, &inbound, process).await;
    (StatusCode::OK, [(CONTENT_TYPE, "text/plain")], ack).into_response()
}
