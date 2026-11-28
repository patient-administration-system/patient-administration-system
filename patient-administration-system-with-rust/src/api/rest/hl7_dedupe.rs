//! The HL7 v2 routes answer a resent message from its first ACK instead of processing it again (NFR-31,
//! HUB-2). This layer reads the message's MSH and runs the handler through [`crate::hl7v2::dedupe::guarded`].
//! First it refuses, with an AR, a message whose MSH-9 is not the type the path handles
//! ([`crate::hl7v2::routing::refusal`], DEV-4): nothing is changed and nothing is remembered, so the same
//! message sent to the right path afterwards is processed.
//! `parse` changes nothing and `batch` is guarded per message inside it, so both pass straight through.

use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{StatusCode, header::CONTENT_TYPE},
    middleware::Next,
    response::{IntoResponse, Response},
};

use crate::api::rest::AppState;
use crate::hl7v2::ack::{AckCode, Inbound, ack_to};
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
    let parsed = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| parse_message(text).ok());
    if let Some(m) = &parsed
        && let Some(why) = crate::hl7v2::routing::refusal(parts.uri.path(), m)
    {
        tracing::warn!(target: "pas::hl7v2", path = parts.uri.path(), "{why}");
        let ack = ack_to(&Inbound::of(m), AckCode::Reject, Some(&why));
        return (StatusCode::OK, [(CONTENT_TYPE, "text/plain")], ack).into_response();
    }
    let Some(inbound) = parsed.map(|m| Inbound::of(&m)) else {
        return next
            .run(Request::from_parts(parts, Body::from(bytes)))
            .await;
    };
    let process = async move {
        let resp = next
            .run(Request::from_parts(parts, Body::from(bytes)))
            .await;
        // The answer could not be read back, so there is no ACK to keep; the error is logged and the sender gets
        // an empty answer, which is no acknowledgement, so it sends again (RCA-IN-6).
        let body = match to_bytes(resp.into_body(), 4 * MAX_BODY).await {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(target: "pas::hl7v2::dedupe", "the ACK could not be read: {e}");
                return String::new();
            }
        };
        String::from_utf8(body.to_vec()).unwrap_or_else(|e| {
            tracing::error!(target: "pas::hl7v2::dedupe", "the ACK is not UTF-8: {e}");
            String::new()
        })
    };
    let ack = crate::hl7v2::dedupe::guarded(&state.db, &inbound, process).await;
    (StatusCode::OK, [(CONTENT_TYPE, "text/plain")], ack).into_response()
}
