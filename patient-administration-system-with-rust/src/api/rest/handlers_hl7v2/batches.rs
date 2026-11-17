//! Part of [`super`]: batches.

use super::*;

pub(super) async fn dispatch_inner_to_ack_body(
    state: &AppState,
    headers: &HeaderMap,
    msg: &crate::hl7v2::Message,
) -> String {
    let body = encode_message(msg);
    let msh = msg.segment("MSH");
    let (code, event) = match msh {
        Some(s) => {
            let field = s.raw(9);
            let mut parts = field.split('^');
            (
                parts.next().unwrap_or("").to_string(),
                parts.next().unwrap_or("").to_string(),
            )
        }
        None => (String::new(), String::new()),
    };
    // Each message in a batch is recognised on its own, as it would be sent alone (NFR-31, HUB-2).
    let inbound = Inbound::of(msg);
    macro_rules! call {
        ($handler:ident) => {{
            crate::hl7v2::dedupe::guarded(&state.db, &inbound, async {
                let resp = $handler(State(state.clone()), headers.clone(), body.clone())
                    .await
                    .into_response();
                let (_, body_box) = resp.into_parts();
                let bytes = axum::body::to_bytes(body_box, 1024 * 1024)
                    .await
                    .unwrap_or_default();
                String::from_utf8(bytes.to_vec()).unwrap_or_default()
            })
            .await
        }};
    }
    match (code.as_str(), event.as_str()) {
        ("ADT", "A01") => call!(admit),
        ("ADT", "A02") => call!(transfer),
        ("ADT", "A03") => call!(discharge),
        ("ADT", "A04") => call!(register),
        ("ADT", "A05") => call!(pre_admit),
        ("ADT", "A06") => call!(change_to_inpatient),
        ("ADT", "A07") => call!(change_to_outpatient),
        ("ADT", "A08") => call!(update),
        ("ADT", "A11") => call!(cancel_admit),
        ("ADT", "A12") => call!(cancel_transfer),
        ("ADT", "A13") => call!(cancel_discharge),
        ("ADT", "A21") => call!(leave_start),
        ("ADT", "A22") => call!(leave_end),
        ("ADT", "A23") => call!(delete_patient),
        ("ADT", "A28") => call!(add_person),
        ("ADT", "A38") => call!(cancel_pre_admit),
        ("ADT", "A40") => call!(merge),
        ("DFT", "P03") => call!(dft),
        ("SIU", "S12") => call!(schedule_book),
        ("SIU", "S13") => call!(schedule_reschedule),
        ("SIU", "S14") => call!(schedule_modify),
        ("SIU", "S15") => call!(schedule_cancel),
        ("MFN", "M02") => call!(mfn_staff),
        ("MFN", "M05") => call!(mfn_location),
        // A type the PAS does not handle is refused, never treated as an add-person (until 2026-11-17 it was).
        _ => ack_to(&inbound, AckCode::Reject, Some("unsupported message type")),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/batch",
    tag = "Hl7",
    summary = "Batch",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; an RFC 9457 problem (`application/problem+json`) with a machine-readable `code`"),
        (status = "5XX", description = "Server or upstream error; an RFC 9457 problem (`application/problem+json`)"),
    )
)]
pub async fn batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let parsed = match parse_batch(&body) {
        Ok(b) => b,
        Err(e) => return ar(&Inbound::unknown(), &e.to_string()),
    };
    let inbound = crate::hl7v2::BatchInbound::of(parsed.bhs.as_ref());
    let mut per_message: Vec<String> = Vec::with_capacity(parsed.messages.len());
    for inner in &parsed.messages {
        per_message.push(dispatch_inner_to_ack_body(&state, &headers, inner).await);
    }
    let envelope = encode_batch_ack(&inbound, &per_message);
    (StatusCode::OK, TEXT_PLAIN, envelope)
}
