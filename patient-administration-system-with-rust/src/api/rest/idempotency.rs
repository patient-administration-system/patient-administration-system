//! `Idempotency-Key` (HUB-8): a POST that carries one is done at most once per caller and key.
//!
//! A client that loses the answer to a booking, a referral or a FHIR `$book` and sends it again would make a
//! second one. With the header, the second request is answered from the first:
//!
//! - **New key:** claimed, the request runs, and its answer is kept (unless it is a server error, 5xx, which is
//!   not final: the claim is removed so a retry runs).
//! - **Same key, same request** (method, path, query and body): the kept answer, with `Idempotent-Replayed:
//!   true`. Nothing runs.
//! - **Same key, different request:** `422`; a key names one request.
//! - **Same key, still running:** `409`; try again shortly.
//!
//! The caller is the verified identity (`x-user-id`, set by the authentication layer), so two callers cannot
//! see each other's answers. A key is 1 to 255 visible ASCII characters. Keys and answers are kept 24 hours and
//! then purged; only hashes of the key and the request are stored, but the answer is kept as it was, so it is
//! listed in the personal-data inventory. A POST without the header is untouched. An answer over 1 MiB, or one that is
//! not UTF-8 text, is not kept (the request is still done once; a retry runs again).

use axum::{
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

use crate::api::rest::AppState;
use crate::db::row::RowExt;
use crate::db::sql::{Conn, Pool, Stmt};

pub const HEADER: &str = "idempotency-key";
pub const REPLAYED: &str = "idempotent-replayed";
const MAX_BODY: usize = 4 * 1024 * 1024;
const MAX_KEPT: usize = 1024 * 1024;

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn problem(status: StatusCode, detail: &str) -> Response {
    crate::api::rest::handlers::error_to_response(match status {
        StatusCode::CONFLICT => crate::Error::conflict(detail.to_string()),
        StatusCode::BAD_REQUEST => crate::Error::validation(detail.to_string()),
        StatusCode::INTERNAL_SERVER_ERROR => crate::Error::internal(detail.to_string()),
        _ => crate::Error::Unprocessable(detail.to_string()),
    })
}

/// Whether `key` is a key this accepts: 1 to 255 visible ASCII characters.
pub fn valid_key(key: &str) -> bool {
    (1..=255).contains(&key.len()) && key.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

pub async fn idempotency_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    if req.method() != Method::POST {
        return next.run(req).await;
    }
    let Some(key) = req
        .headers()
        .get(HEADER)
        .map(|v| v.to_str().map(str::to_string))
    else {
        return next.run(req).await;
    };
    let Ok(key) = key else {
        return problem(
            StatusCode::BAD_REQUEST,
            "Idempotency-Key must be visible ASCII",
        );
    };
    if !valid_key(&key) {
        return problem(
            StatusCode::BAD_REQUEST,
            "Idempotency-Key must be 1 to 255 visible ASCII characters",
        );
    }
    // No caller header is the anonymous caller; one that cannot be read is refused, not merged into it.
    let caller = match req.headers().get("x-user-id") {
        None => String::new(),
        Some(v) => match v.to_str() {
            Ok(text) => text.to_string(),
            Err(_) => {
                return problem(StatusCode::BAD_REQUEST, "x-user-id must be visible ASCII");
            }
        },
    };
    let (parts, body) = req.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_BODY).await else {
        return problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "the request body is too large",
        );
    };
    let mut fingerprint = Vec::with_capacity(bytes.len());
    fingerprint.extend_from_slice(parts.method.as_str().as_bytes());
    fingerprint.push(b' ');
    fingerprint.extend_from_slice(parts.uri.to_string().as_bytes());
    fingerprint.push(b'\n');
    fingerprint.extend_from_slice(&bytes);
    let request_hash = hex(&fingerprint);
    let key_hash = hex(key.as_bytes());
    let db = &state.db;
    match claim(db, &caller, &key_hash, &request_hash).await {
        Ok(Claimed::New) => {}
        Ok(Claimed::Replay {
            status,
            content_type,
            body,
        }) => {
            let mut resp = Response::new(Body::from(body));
            *resp.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
            if let Some(ct) = content_type.and_then(|c| HeaderValue::from_str(&c).ok()) {
                resp.headers_mut().insert(header::CONTENT_TYPE, ct);
            }
            resp.headers_mut()
                .insert(REPLAYED, HeaderValue::from_static("true"));
            return resp;
        }
        Ok(Claimed::OtherRequest) => {
            return problem(
                StatusCode::UNPROCESSABLE_ENTITY,
                "this Idempotency-Key was used for a different request",
            );
        }
        Ok(Claimed::Running) => {
            return problem(
                StatusCode::CONFLICT,
                "a request with this Idempotency-Key is still running",
            );
        }
        Err(e) => return crate::api::rest::handlers::error_to_response(e),
    }
    let resp = next
        .run(Request::from_parts(parts, Body::from(bytes)))
        .await;
    let status = resp.status();
    let (resp_parts, resp_body) = resp.into_parts();
    let Ok(body) = to_bytes(resp_body, MAX_BODY).await else {
        // The answer could not be read back to be kept, so it is not kept and the claim is released.
        if let Err(e) = release(db, &caller, &key_hash).await {
            tracing::error!(target: "pas::idempotency", "could not release the key: {e}");
        }
        return problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the response could not be read",
        );
    };
    let text = std::str::from_utf8(&body).ok().map(str::to_string);
    let keep = !status.is_server_error() && body.len() <= MAX_KEPT && text.is_some();
    let content_type = resp_parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let settled = if keep {
        finish(
            db,
            &caller,
            &key_hash,
            status.as_u16(),
            content_type,
            // `keep` is true only when the text is present.
            text.unwrap_or(String::new()),
        )
        .await
    } else {
        release(db, &caller, &key_hash).await
    };
    if let Err(e) = settled {
        tracing::error!(target: "pas::idempotency", "could not record the answer: {e}");
    }
    Response::from_parts(resp_parts, Body::from(body)).into_response()
}

enum Claimed {
    New,
    Replay {
        status: u16,
        content_type: Option<String>,
        body: String,
    },
    OtherRequest,
    Running,
}

async fn claim(db: &Pool, caller: &str, key: &str, request: &str) -> crate::Result<Claimed> {
    db.run(Stmt::new(
        "DELETE FROM idempotency_keys WHERE caller = $1 AND key_sha256 = $2 AND created_at < now() - interval '24 hours'",
        [caller.into(), key.into()],
    ))
    .await?;
    let new = db
        .one(Stmt::new(
            "INSERT INTO idempotency_keys (caller, key_sha256, request_sha256, state) VALUES ($1, $2, $3, 'processing') \
             ON CONFLICT DO NOTHING RETURNING state",
            [caller.into(), key.into(), request.into()],
        ))
        .await?;
    if new.is_some() {
        return Ok(Claimed::New);
    }
    let Some(row) = db
        .one(Stmt::new(
            "SELECT request_sha256, state, status, content_type, body FROM idempotency_keys \
             WHERE caller = $1 AND key_sha256 = $2",
            [caller.into(), key.into()],
        ))
        .await?
    else {
        return Ok(Claimed::Running);
    };
    let stored: String = row.col("request_sha256")?;
    if stored != request {
        return Ok(Claimed::OtherRequest);
    }
    let state: String = row.col("state")?;
    if state != "done" {
        return Ok(Claimed::Running);
    }
    // A finished key always has the status it was answered with; anything else is a corrupt record, which is
    // reported, not replayed as a made-up 200 (RCA-IN-6).
    let status = row
        .col::<Option<i32>>("status")?
        .and_then(|s| u16::try_from(s).ok())
        .ok_or_else(|| crate::Error::internal("a finished idempotency key has no valid status"))?;
    Ok(Claimed::Replay {
        status,
        content_type: row.col("content_type")?,
        // RCA-DECL-1: a kept answer with no body (an empty one) is replayed empty.
        body: row.col::<Option<String>>("body")?.unwrap_or(String::new()),
    })
}

async fn finish(
    db: &Pool,
    caller: &str,
    key: &str,
    status: u16,
    content_type: Option<String>,
    body: String,
) -> crate::Result<()> {
    db.run(Stmt::new(
        "UPDATE idempotency_keys SET state = 'done', status = $3, content_type = $4, body = $5 \
         WHERE caller = $1 AND key_sha256 = $2",
        [
            caller.into(),
            key.into(),
            i32::from(status).into(),
            content_type.into(),
            body.into(),
        ],
    ))
    .await?;
    Ok(())
}

async fn release(db: &Pool, caller: &str, key: &str) -> crate::Result<()> {
    db.run(Stmt::new(
        "DELETE FROM idempotency_keys WHERE caller = $1 AND key_sha256 = $2",
        [caller.into(), key.into()],
    ))
    .await?;
    Ok(())
}

/// Remove keys older than 24 hours. Returns how many.
pub async fn purge(db: &Pool) -> crate::Result<u64> {
    Ok(db
        .run(Stmt::text(
            "DELETE FROM idempotency_keys WHERE created_at < now() - interval '24 hours'",
        ))
        .await?
        .rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_one_to_255_visible_ascii_characters() {
        assert!(valid_key("a"));
        assert!(valid_key("8e03978e-40d5-43e8-bc93-6894a57f9324"));
        assert!(valid_key(&"k".repeat(255)));
        for bad in ["", " ", "has space", "tab\there", "é", &"k".repeat(256)] {
            assert!(!valid_key(bad), "{bad:?}");
        }
    }
}
