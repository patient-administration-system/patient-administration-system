//! REST handlers for the write-back API (CAP-2): a patient's reply changes the appointment now.

use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, header},
};
use serde_json::Value;

use crate::Error;
use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::db::repositories::audit::UserContext as RepoUserContext;
use crate::writeback::{MAX_BATCH, ReplyRequest};

fn audit_ctx(headers: &HeaderMap) -> RepoUserContext {
    let uc = user_context_from_headers(headers);
    RepoUserContext {
        user_id: uc.user_id,
        user_ip: uc.user_ip,
        user_agent: uc.user_agent,
    }
}

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// `POST /api/write-back/replies`: apply one reply. A reply that cannot be applied for a
/// deterministic reason is still `200` with `status: "rejected"` and a `reject_reason`: the sender
/// should not retry it. Malformed input is `400`; a reused `message_id` is `409`.
pub async fn post_reply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ReplyRequest>,
) -> HandlerResult {
    match state.writeback.apply(req, &audit_ctx(&headers)).await {
        Ok(r) => ok(serde_json::to_value(r).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/write-back/batch`: the fallback when replies arrive as a file. The body is a JSON
/// array of replies, or CSV (`Content-Type: text/csv`) with the header
/// `message_id,action,appointment_id,person_id,channel,barrier_code,new_slot_id,received_at`.
/// Each row is applied independently and reported; at most 1000 rows.
pub async fn post_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> HandlerResult {
    let is_csv = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/csv"));
    let parsed: Result<Vec<ReplyRequest>, Error> = if is_csv {
        parse_csv(&body)
    } else {
        serde_json::from_slice(&body)
            .map_err(|e| Error::validation(format!("body is not a JSON array of replies: {e}")))
    };
    let reqs = match parsed {
        Ok(r) => r,
        Err(e) => return error_to_response(e),
    };
    match state
        .writeback
        .apply_batch(reqs, &audit_ctx(&headers))
        .await
    {
        Ok(items) => ok(serde_json::to_value(items).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// Parse a CSV batch. Blank optional columns are absent; an unreadable row fails the whole file,
/// because a half-understood file is worse than a refused one.
pub fn parse_csv(body: &[u8]) -> Result<Vec<ReplyRequest>, Error> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(body);
    let mut out = Vec::new();
    for (i, row) in reader.deserialize::<ReplyRequest>().enumerate() {
        if out.len() >= MAX_BATCH {
            return Err(Error::validation(format!(
                "a batch may carry at most {MAX_BATCH} replies"
            )));
        }
        out.push(row.map_err(|e| Error::validation(format!("CSV row {}: {e}", i + 1)))?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER: &str =
        "message_id,action,appointment_id,person_id,channel,barrier_code,new_slot_id,received_at\n";
    const A: &str = "11111111-1111-1111-1111-111111111111";
    const P: &str = "22222222-2222-2222-2222-222222222222";
    const S: &str = "33333333-3333-3333-3333-333333333333";

    #[test]
    fn test_csv_parses_each_action_and_leaves_blank_columns_absent() {
        let csv = format!(
            "{HEADER}m1,confirm,{A},{P},sms,,,\nm2,cancel,{A},{P},whatsapp,work,,2026-10-18T09:00:00Z\nm3,rebook,{A},{P},sms,,{S},\n"
        );
        let rows = parse_csv(csv.as_bytes()).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].barrier_code.is_none() && rows[0].new_slot_id.is_none());
        assert!(rows[1].barrier_code.is_some() && rows[1].received_at.is_some());
        assert_eq!(
            rows[2].new_slot_id.map(|u| u.to_string()).as_deref(),
            Some(S)
        );
    }

    #[test]
    fn test_csv_refuses_the_whole_file_on_one_bad_row() {
        let csv = format!("{HEADER}m1,confirm,{A},{P},sms,,,\nm2,explode,{A},{P},sms,,,\n");
        let err = parse_csv(csv.as_bytes()).unwrap_err().to_string();
        assert!(err.contains("row 2"), "{err}");
    }

    #[test]
    fn test_csv_refuses_an_unknown_barrier_code() {
        let csv = format!("{HEADER}m1,cancel,{A},{P},sms,too_busy,,\n");
        assert!(parse_csv(csv.as_bytes()).is_err());
    }
}
