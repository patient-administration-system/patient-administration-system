//! [`PersonClient`] trait + [`HttpPersonClient`] reqwest impl +
//! [`StubPersonClient`] in-memory impl for tests.
//!
//! See [`crate::upstream`] module docs for the design rules.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::upstream::{MergeOutcome, UpstreamAuditEntry, UserContext};
use crate::{Error, Result};

mod http;
mod stub;
pub use http::*;
pub use stub::*;

/// One `Person` row as returned by the upstream. The id is typed; every
/// other field rides in `data` as raw JSON so the upstream wire shape can
/// evolve without breaking PAS compile.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Person {
    pub id: Uuid,
    /// Every other field as returned by the upstream (`name`, `gender`,
    /// `birth_date`, `identifiers`, `addresses`, `telecom`, `active`,
    /// `replaced_by`, …).
    #[serde(flatten)]
    pub data: serde_json::Map<String, serde_json::Value>,
}

/// POST/PUT body for a person mutation. Same flat-JSON shape as [`Person`]
/// minus the id (the upstream allocates it on create, and PUTs use the
/// URL id rather than a body id).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PersonCreate {
    #[serde(flatten)]
    pub fields: serde_json::Map<String, serde_json::Value>,
}

/// One candidate returned by the upstream `/api/persons/match` or
/// `/api/persons/check-duplicates` endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PersonMatch {
    pub person: Person,
    /// Match score, 0.0–1.0. Higher is more confident.
    pub score: f64,
}

/// Async client surface for `person-service`.
///
/// Every method takes a [`UserContext`] so the upstream's audit log credits
/// the original PAS-side caller (not the PAS service account). Errors are
/// the standard [`crate::Error`] variants — see the module docs for the
/// translation rules.
#[async_trait]
pub trait PersonClient: Send + Sync {
    async fn create(&self, person: PersonCreate, ctx: &UserContext) -> Result<Person>;
    async fn get(&self, id: Uuid, ctx: &UserContext) -> Result<Person>;
    async fn update(&self, id: Uuid, person: PersonCreate, ctx: &UserContext) -> Result<Person>;
    async fn delete(&self, id: Uuid, ctx: &UserContext) -> Result<()>;
    async fn search(&self, query: &str, limit: u32, ctx: &UserContext) -> Result<Vec<Person>>;
    /// `POST /api/persons/match` — fuzzy match against the existing
    /// population. Returns candidates ordered by descending score.
    async fn match_persons(
        &self,
        candidate: PersonCreate,
        ctx: &UserContext,
    ) -> Result<Vec<PersonMatch>>;
    /// `POST /api/persons/check-duplicates` — same shape as `match_persons`
    /// but conventionally used pre-create to short-circuit registration.
    async fn check_duplicates(
        &self,
        candidate: PersonCreate,
        ctx: &UserContext,
    ) -> Result<Vec<PersonMatch>>;
    async fn merge(&self, source: Uuid, target: Uuid, ctx: &UserContext) -> Result<MergeOutcome>;
    async fn masked(&self, id: Uuid, ctx: &UserContext) -> Result<Person>;
    async fn export(&self, id: Uuid, ctx: &UserContext) -> Result<serde_json::Value>;
    async fn audit(
        &self,
        id: Uuid,
        limit: u32,
        ctx: &UserContext,
    ) -> Result<Vec<UpstreamAuditEntry>>;
    /// `GET /api/persons/export.{format}` — bulk export. `format` is one
    /// of `json` / `xml` / `tsv` / `csv`. Returns `(content_type, body_bytes)`
    /// so PAS can stream the upstream response back to the caller verbatim
    /// without round-tripping through a typed shape.
    async fn bulk_export(&self, format: &str, ctx: &UserContext) -> Result<(String, Vec<u8>)>;
    /// `POST /api/persons/import` — bulk import. The caller supplies the
    /// `content_type` (which the upstream uses to pick a parser) and the
    /// raw body bytes. Returns the upstream's JSON summary (counts, error
    /// rows, etc.) as a generic `Value` so PAS doesn't have to model the
    /// upstream's import-report shape.
    async fn bulk_import(
        &self,
        content_type: &str,
        body: Vec<u8>,
        ctx: &UserContext,
    ) -> Result<serde_json::Value>;
}

// ─── HTTP impl ──────────────────────────────────────────────────────────────

// ─── Stub impl for tests ────────────────────────────────────────────────────

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn ctx() -> UserContext {
        UserContext {
            user_id: Some("alice".into()),
            user_ip: Some("10.0.0.1".into()),
            user_agent: Some("test/1.0".into()),
        }
    }

    fn make_person_create() -> PersonCreate {
        let mut fields = serde_json::Map::new();
        fields.insert(
            "name".into(),
            serde_json::json!({ "family": "Doe", "given": ["Jane"] }),
        );
        fields.insert("gender".into(), serde_json::json!("female"));
        PersonCreate { fields }
    }

    // ─── Stub impl tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn stub_create_then_get_roundtrip() {
        let c = StubPersonClient::new();
        let p = c.create(make_person_create(), &ctx()).await.unwrap();
        assert_eq!(c.len(), 1);
        let got = c.get(p.id, &ctx()).await.unwrap();
        assert_eq!(got.id, p.id);
        assert_eq!(
            got.data.get("gender").and_then(|v| v.as_str()),
            Some("female")
        );
    }

    #[tokio::test]
    async fn stub_get_unknown_returns_not_found() {
        let c = StubPersonClient::new();
        let err = c.get(Uuid::new_v4(), &ctx()).await.unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[tokio::test]
    async fn stub_update_unknown_returns_not_found() {
        let c = StubPersonClient::new();
        let err = c
            .update(Uuid::new_v4(), make_person_create(), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[tokio::test]
    async fn stub_delete_removes_then_get_404s() {
        let c = StubPersonClient::new();
        let p = c.create(make_person_create(), &ctx()).await.unwrap();
        c.delete(p.id, &ctx()).await.unwrap();
        assert!(c.is_empty());
        let err = c.get(p.id, &ctx()).await.unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[tokio::test]
    async fn stub_merge_drops_source_keeps_target() {
        let c = StubPersonClient::new();
        let a = c.create(make_person_create(), &ctx()).await.unwrap();
        let b = c.create(make_person_create(), &ctx()).await.unwrap();
        let out = c.merge(a.id, b.id, &ctx()).await.unwrap();
        assert_eq!(out.source_id, a.id);
        assert_eq!(out.target_id, b.id);
        assert!(matches!(
            c.get(a.id, &ctx()).await.unwrap_err(),
            Error::NotFound(_)
        ));
        assert!(c.get(b.id, &ctx()).await.is_ok());
    }

    #[tokio::test]
    async fn stub_merge_self_is_validation() {
        let c = StubPersonClient::new();
        let a = c.create(make_person_create(), &ctx()).await.unwrap();
        let err = c.merge(a.id, a.id, &ctx()).await.unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[tokio::test]
    async fn stub_search_returns_up_to_limit() {
        let c = StubPersonClient::new();
        for _ in 0..5 {
            c.create(make_person_create(), &ctx()).await.unwrap();
        }
        let got = c.search("anything", 3, &ctx()).await.unwrap();
        assert_eq!(got.len(), 3);
    }

    #[tokio::test]
    async fn stub_bulk_export_json_emits_array() {
        let c = StubPersonClient::new();
        c.create(make_person_create(), &ctx()).await.unwrap();
        c.create(make_person_create(), &ctx()).await.unwrap();
        let (ct, body) = c.bulk_export("json", &ctx()).await.unwrap();
        assert_eq!(ct, "application/json");
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v.as_array().is_some_and(|a| a.len() == 2));
    }

    #[tokio::test]
    async fn stub_bulk_export_unsupported_format_is_validation() {
        let c = StubPersonClient::new();
        let err = c.bulk_export("yaml", &ctx()).await.unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[tokio::test]
    async fn stub_bulk_export_xml_returns_xml_content_type() {
        let c = StubPersonClient::new();
        let (ct, _) = c.bulk_export("xml", &ctx()).await.unwrap();
        assert_eq!(ct, "application/xml");
    }

    #[tokio::test]
    async fn stub_bulk_import_returns_report_value() {
        let c = StubPersonClient::new();
        let v = c
            .bulk_import("text/csv", b"family,given\nDoe,Jane\n".to_vec(), &ctx())
            .await
            .unwrap();
        assert!(v.is_object());
        assert!(v.get("imported").is_some());
    }

    // ─── HTTP impl tests against an in-process TcpListener stub server ─

    /// Spawn a one-shot HTTP responder on an ephemeral port. Reads the
    /// request, sends `response` (raw HTTP/1.1 wire bytes) back, closes.
    /// Returns the bound `host:port`.
    async fn one_shot_server(response: &'static [u8]) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(response).await;
                let _ = sock.shutdown().await;
            }
        });
        addr
    }

    /// Capture the inbound request bytes so a test can assert on headers /
    /// body. Returns `(addr, request_bytes_channel)`.
    async fn capturing_server(
        response: &'static [u8],
    ) -> (SocketAddr, tokio::sync::oneshot::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                buf.truncate(n);
                let _ = tx.send(buf);
                let _ = sock.write_all(response).await;
                let _ = sock.shutdown().await;
            }
        });
        (addr, rx)
    }

    fn http_envelope_ok(body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .into_bytes()
    }

    fn http_envelope_status(status_line: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .into_bytes()
    }

    // The `one_shot_server` helper takes a `'static` slice, so leak the
    // formatted body so we can hand it over.
    fn leak(bytes: Vec<u8>) -> &'static [u8] {
        Box::leak(bytes.into_boxed_slice())
    }

    #[tokio::test]
    async fn http_get_unwraps_envelope() {
        let id = Uuid::new_v4();
        let body = format!(
            r#"{{"success":true,"data":{{"id":"{id}","name":{{"family":"Doe"}}}},"error":null}}"#
        );
        let resp = leak(http_envelope_ok(&body));
        let addr = one_shot_server(resp).await;
        let client =
            HttpPersonClient::new(format!("http://{addr}"), Duration::from_secs(2), None).unwrap();
        let p = client.get(id, &ctx()).await.unwrap();
        assert_eq!(p.id, id);
        assert_eq!(
            p.data
                .get("name")
                .and_then(|v| v.get("family"))
                .and_then(|v| v.as_str()),
            Some("Doe")
        );
    }

    #[tokio::test]
    async fn http_get_404_envelope_becomes_not_found() {
        let body = r#"{"success":false,"data":null,"error":{"code":"NOT_FOUND","message":"no such person"}}"#;
        let resp = leak(http_envelope_status("404 Not Found", body));
        let addr = one_shot_server(resp).await;
        let client =
            HttpPersonClient::new(format!("http://{addr}"), Duration::from_secs(2), None).unwrap();
        let err = client.get(Uuid::new_v4(), &ctx()).await.unwrap_err();
        assert!(
            matches!(err, Error::NotFound(_)),
            "expected NotFound, got {err:?}"
        );
    }

    #[tokio::test]
    async fn http_get_400_envelope_becomes_validation() {
        let body =
            r#"{"success":false,"data":null,"error":{"code":"VALIDATION","message":"bad id"}}"#;
        let resp = leak(http_envelope_status("400 Bad Request", body));
        let addr = one_shot_server(resp).await;
        let client =
            HttpPersonClient::new(format!("http://{addr}"), Duration::from_secs(2), None).unwrap();
        let err = client.get(Uuid::new_v4(), &ctx()).await.unwrap_err();
        assert!(
            matches!(err, Error::Validation(_)),
            "expected Validation, got {err:?}"
        );
    }

    #[tokio::test]
    async fn http_get_409_envelope_becomes_conflict() {
        let body = r#"{"success":false,"data":null,"error":{"code":"CONFLICT","message":"already merged"}}"#;
        let resp = leak(http_envelope_status("409 Conflict", body));
        let addr = one_shot_server(resp).await;
        let client =
            HttpPersonClient::new(format!("http://{addr}"), Duration::from_secs(2), None).unwrap();
        let err = client.get(Uuid::new_v4(), &ctx()).await.unwrap_err();
        assert!(
            matches!(err, Error::Conflict(_)),
            "expected Conflict, got {err:?}"
        );
    }

    #[tokio::test]
    async fn http_get_500_no_envelope_becomes_upstream() {
        let resp = leak(http_envelope_status("500 Internal Server Error", "boom"));
        let addr = one_shot_server(resp).await;
        let client =
            HttpPersonClient::new(format!("http://{addr}"), Duration::from_secs(2), None).unwrap();
        let err = client.get(Uuid::new_v4(), &ctx()).await.unwrap_err();
        assert!(matches!(err, Error::Upstream(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn http_transport_failure_becomes_upstream() {
        // Bind a listener but drop it immediately so the port is free —
        // connecting to it will get refused.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let client =
            HttpPersonClient::new(format!("http://{addr}"), Duration::from_millis(500), None)
                .unwrap();
        let err = client.get(Uuid::new_v4(), &ctx()).await.unwrap_err();
        assert!(matches!(err, Error::Upstream(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn http_attaches_bearer_and_user_headers() {
        let id = Uuid::new_v4();
        let body = format!(r#"{{"success":true,"data":{{"id":"{id}"}},"error":null}}"#);
        let resp = leak(http_envelope_ok(&body));
        let (addr, rx) = capturing_server(resp).await;
        let client = HttpPersonClient::new(
            format!("http://{addr}"),
            Duration::from_secs(2),
            Some("secret-token".into()),
        )
        .unwrap();
        let _ = client.get(id, &ctx()).await.unwrap();
        let raw = rx.await.unwrap();
        // Header names are case-insensitive in HTTP; reqwest lower-cases
        // them on the wire. Lower-case the whole request before checking
        // so the assertion does not couple to that detail.
        let req = String::from_utf8_lossy(&raw).to_ascii_lowercase();
        assert!(
            req.contains("authorization: bearer secret-token"),
            "bearer header missing: {req}"
        );
        assert!(
            req.contains("x-user-id: alice"),
            "user id header missing: {req}"
        );
        assert!(
            req.contains("x-user-ip: 10.0.0.1"),
            "user ip header missing: {req}"
        );
    }
}
