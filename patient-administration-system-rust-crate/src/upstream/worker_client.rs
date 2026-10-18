//! [`WorkerClient`] trait + [`HttpWorkerClient`] reqwest impl +
//! [`StubWorkerClient`] in-memory impl for tests.
//!
//! Mirror of `person_client.rs` against `worker-service`. The two
//! upstream services share an envelope shape and a CRUD/search/match/merge
//! surface; the only difference is the resource name (`Person` ↔ `Worker`)
//! and the URL prefix (`/api/persons` ↔ `/api/workers`).
//!
//! See [`crate::upstream`] module docs for the design rules. See
//! [`crate::upstream::person_client`] for inline commentary on every
//! design decision — this file applies the same patterns without repeating
//! the rationale.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::upstream::{MergeOutcome, UpstreamAuditEntry, UserContext};
use crate::{Error, Result};

/// One `Worker` row as returned by the upstream. The id is typed; every
/// other field rides in `data` as raw JSON so the upstream wire shape can
/// evolve without breaking PAS compile.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Worker {
    pub id: Uuid,
    #[serde(flatten)]
    pub data: serde_json::Map<String, serde_json::Value>,
}

/// POST/PUT body for a worker mutation.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct WorkerCreate {
    #[serde(flatten)]
    pub fields: serde_json::Map<String, serde_json::Value>,
}

/// One candidate returned by the upstream `/api/workers/match` or
/// `/api/workers/check-duplicates` endpoint.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WorkerMatch {
    pub worker: Worker,
    pub score: f64,
}

/// Async client surface for `worker-service`.
#[async_trait]
pub trait WorkerClient: Send + Sync {
    async fn create(&self, worker: WorkerCreate, ctx: &UserContext) -> Result<Worker>;
    async fn get(&self, id: Uuid, ctx: &UserContext) -> Result<Worker>;
    async fn update(&self, id: Uuid, worker: WorkerCreate, ctx: &UserContext) -> Result<Worker>;
    async fn delete(&self, id: Uuid, ctx: &UserContext) -> Result<()>;
    async fn search(&self, query: &str, limit: u32, ctx: &UserContext) -> Result<Vec<Worker>>;
    async fn match_workers(
        &self,
        candidate: WorkerCreate,
        ctx: &UserContext,
    ) -> Result<Vec<WorkerMatch>>;
    async fn check_duplicates(
        &self,
        candidate: WorkerCreate,
        ctx: &UserContext,
    ) -> Result<Vec<WorkerMatch>>;
    async fn merge(&self, source: Uuid, target: Uuid, ctx: &UserContext) -> Result<MergeOutcome>;
    async fn masked(&self, id: Uuid, ctx: &UserContext) -> Result<Worker>;
    async fn export(&self, id: Uuid, ctx: &UserContext) -> Result<serde_json::Value>;
    async fn audit(
        &self,
        id: Uuid,
        limit: u32,
        ctx: &UserContext,
    ) -> Result<Vec<UpstreamAuditEntry>>;
}

// ─── HTTP impl ──────────────────────────────────────────────────────────────

pub struct HttpWorkerClient {
    http: reqwest::Client,
    base_url: String,
    bearer: Option<String>,
}

impl HttpWorkerClient {
    pub fn new(base_url: String, timeout: Duration, bearer: Option<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| Error::upstream(format!("reqwest build: {e}")))?;
        let base_url = base_url.trim_end_matches('/').to_string();
        Ok(Self {
            http,
            base_url,
            bearer,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    fn enrich(
        &self,
        mut rb: reqwest::RequestBuilder,
        ctx: &UserContext,
    ) -> reqwest::RequestBuilder {
        if let Some(token) = &self.bearer {
            rb = rb.bearer_auth(token);
        }
        if let Some(v) = &ctx.user_id {
            rb = rb.header("X-User-Id", v);
        }
        if let Some(v) = &ctx.user_ip {
            rb = rb.header("X-User-Ip", v);
        }
        if let Some(v) = &ctx.user_agent {
            rb = rb.header("X-User-Agent", v);
        }
        rb
    }
}

#[derive(Debug, Deserialize)]
struct UpstreamEnvelope<T> {
    success: bool,
    data: Option<T>,
    error: Option<UpstreamError>,
}

#[derive(Debug, Deserialize)]
struct UpstreamError {
    code: String,
    message: String,
}

fn translate_envelope_error(status: reqwest::StatusCode, err: UpstreamError) -> Error {
    let UpstreamError { code, message } = err;
    let msg = format!("upstream {code}: {message}");
    match status.as_u16() {
        404 => Error::NotFound(msg),
        400 | 422 => Error::Validation(msg),
        409 => Error::Conflict(msg),
        _ => Error::Upstream(msg),
    }
}

async fn send_envelope<T: serde::de::DeserializeOwned>(rb: reqwest::RequestBuilder) -> Result<T> {
    let resp = rb
        .send()
        .await
        .map_err(|e| Error::upstream(format!("transport: {e}")))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| Error::upstream(format!("read body: {e}")))?;
    let env: UpstreamEnvelope<T> = serde_json::from_str(&body).map_err(|e| {
        Error::upstream(format!(
            "malformed envelope ({status}): {e}; body bytes={}",
            body.len()
        ))
    })?;
    match (env.success, env.data, env.error) {
        (true, Some(data), _) => Ok(data),
        (false, _, Some(err)) => Err(translate_envelope_error(status, err)),
        (success, data, error) => Err(Error::upstream(format!(
            "envelope had success={success} data_present={} error_present={}",
            data.is_some(),
            error.is_some()
        ))),
    }
}

async fn send_envelope_unit(rb: reqwest::RequestBuilder) -> Result<()> {
    let resp = rb
        .send()
        .await
        .map_err(|e| Error::upstream(format!("transport: {e}")))?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let body = resp
        .text()
        .await
        .map_err(|e| Error::upstream(format!("read body: {e}")))?;
    if let Ok(env) = serde_json::from_str::<UpstreamEnvelope<serde_json::Value>>(&body)
        && let Some(err) = env.error
    {
        return Err(translate_envelope_error(status, err));
    }
    Err(Error::upstream(format!(
        "non-2xx status {status}: body bytes={}",
        body.len()
    )))
}

#[async_trait]
impl WorkerClient for HttpWorkerClient {
    async fn create(&self, worker: WorkerCreate, ctx: &UserContext) -> Result<Worker> {
        let rb = self.enrich(self.http.post(self.url("/api/workers")).json(&worker), ctx);
        send_envelope(rb).await
    }

    async fn get(&self, id: Uuid, ctx: &UserContext) -> Result<Worker> {
        let rb = self.enrich(self.http.get(self.url(&format!("/api/workers/{id}"))), ctx);
        send_envelope(rb).await
    }

    async fn update(&self, id: Uuid, worker: WorkerCreate, ctx: &UserContext) -> Result<Worker> {
        let rb = self.enrich(
            self.http
                .put(self.url(&format!("/api/workers/{id}")))
                .json(&worker),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn delete(&self, id: Uuid, ctx: &UserContext) -> Result<()> {
        let rb = self.enrich(
            self.http.delete(self.url(&format!("/api/workers/{id}"))),
            ctx,
        );
        send_envelope_unit(rb).await
    }

    async fn search(&self, query: &str, limit: u32, ctx: &UserContext) -> Result<Vec<Worker>> {
        let rb = self.enrich(
            self.http
                .get(self.url("/api/workers/search"))
                .query(&[("q", query), ("limit", &limit.to_string())]),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn match_workers(
        &self,
        candidate: WorkerCreate,
        ctx: &UserContext,
    ) -> Result<Vec<WorkerMatch>> {
        let rb = self.enrich(
            self.http
                .post(self.url("/api/workers/match"))
                .json(&candidate),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn check_duplicates(
        &self,
        candidate: WorkerCreate,
        ctx: &UserContext,
    ) -> Result<Vec<WorkerMatch>> {
        let rb = self.enrich(
            self.http
                .post(self.url("/api/workers/check-duplicates"))
                .json(&candidate),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn merge(&self, source: Uuid, target: Uuid, ctx: &UserContext) -> Result<MergeOutcome> {
        let body = serde_json::json!({ "source_id": source, "target_id": target });
        let rb = self.enrich(
            self.http.post(self.url("/api/workers/merge")).json(&body),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn masked(&self, id: Uuid, ctx: &UserContext) -> Result<Worker> {
        let rb = self.enrich(
            self.http
                .get(self.url(&format!("/api/workers/{id}/masked"))),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn export(&self, id: Uuid, ctx: &UserContext) -> Result<serde_json::Value> {
        let rb = self.enrich(
            self.http
                .get(self.url(&format!("/api/workers/{id}/export"))),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn audit(
        &self,
        id: Uuid,
        limit: u32,
        ctx: &UserContext,
    ) -> Result<Vec<UpstreamAuditEntry>> {
        let rb = self.enrich(
            self.http
                .get(self.url(&format!("/api/workers/{id}/audit")))
                .query(&[("limit", &limit.to_string())]),
            ctx,
        );
        send_envelope(rb).await
    }
}

// ─── Stub impl for tests ────────────────────────────────────────────────────

/// In-memory [`WorkerClient`] for tests. Zero HTTP.
pub struct StubWorkerClient {
    rows: Mutex<HashMap<Uuid, Worker>>,
}

impl Default for StubWorkerClient {
    fn default() -> Self {
        Self::new()
    }
}

impl StubWorkerClient {
    pub fn new() -> Self {
        Self {
            rows: Mutex::new(HashMap::new()),
        }
    }

    pub fn insert(&self, worker: Worker) {
        self.rows.lock().unwrap().insert(worker.id, worker);
    }

    pub fn len(&self) -> usize {
        self.rows.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.lock().unwrap().is_empty()
    }
}

#[async_trait]
impl WorkerClient for StubWorkerClient {
    async fn create(&self, worker: WorkerCreate, _ctx: &UserContext) -> Result<Worker> {
        let id = Uuid::new_v4();
        let new = Worker {
            id,
            data: worker.fields,
        };
        self.rows.lock().unwrap().insert(id, new.clone());
        Ok(new)
    }

    async fn get(&self, id: Uuid, _ctx: &UserContext) -> Result<Worker> {
        self.rows
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| Error::not_found(format!("worker {id}")))
    }

    async fn update(&self, id: Uuid, worker: WorkerCreate, _ctx: &UserContext) -> Result<Worker> {
        let mut rows = self.rows.lock().unwrap();
        if !rows.contains_key(&id) {
            return Err(Error::not_found(format!("worker {id}")));
        }
        let updated = Worker {
            id,
            data: worker.fields,
        };
        rows.insert(id, updated.clone());
        Ok(updated)
    }

    async fn delete(&self, id: Uuid, _ctx: &UserContext) -> Result<()> {
        let removed = self.rows.lock().unwrap().remove(&id);
        match removed {
            Some(_) => Ok(()),
            None => Err(Error::not_found(format!("worker {id}"))),
        }
    }

    async fn search(&self, _query: &str, limit: u32, _ctx: &UserContext) -> Result<Vec<Worker>> {
        let rows = self.rows.lock().unwrap();
        Ok(rows.values().take(limit as usize).cloned().collect())
    }

    async fn match_workers(
        &self,
        _candidate: WorkerCreate,
        _ctx: &UserContext,
    ) -> Result<Vec<WorkerMatch>> {
        Ok(Vec::new())
    }

    async fn check_duplicates(
        &self,
        _candidate: WorkerCreate,
        _ctx: &UserContext,
    ) -> Result<Vec<WorkerMatch>> {
        Ok(Vec::new())
    }

    async fn merge(&self, source: Uuid, target: Uuid, _ctx: &UserContext) -> Result<MergeOutcome> {
        let mut rows = self.rows.lock().unwrap();
        if !rows.contains_key(&source) {
            return Err(Error::not_found(format!("worker {source}")));
        }
        if !rows.contains_key(&target) {
            return Err(Error::not_found(format!("worker {target}")));
        }
        if source == target {
            return Err(Error::validation("self-merge"));
        }
        rows.remove(&source);
        Ok(MergeOutcome {
            source_id: source,
            target_id: target,
        })
    }

    async fn masked(&self, id: Uuid, ctx: &UserContext) -> Result<Worker> {
        self.get(id, ctx).await
    }

    async fn export(&self, id: Uuid, ctx: &UserContext) -> Result<serde_json::Value> {
        let w = self.get(id, ctx).await?;
        Ok(serde_json::to_value(w).unwrap_or(serde_json::Value::Null))
    }

    async fn audit(
        &self,
        _id: Uuid,
        _limit: u32,
        _ctx: &UserContext,
    ) -> Result<Vec<UpstreamAuditEntry>> {
        Ok(Vec::new())
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn ctx() -> UserContext {
        UserContext {
            user_id: Some("bob".into()),
            user_ip: Some("10.0.0.2".into()),
            user_agent: Some("test/1.0".into()),
        }
    }

    fn make_worker_create() -> WorkerCreate {
        let mut fields = serde_json::Map::new();
        fields.insert(
            "name".into(),
            serde_json::json!({ "family": "Smith", "given": ["Pat"] }),
        );
        fields.insert("active".into(), serde_json::json!(true));
        WorkerCreate { fields }
    }

    // ─── Stub impl tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn stub_create_then_get_roundtrip() {
        let c = StubWorkerClient::new();
        let w = c.create(make_worker_create(), &ctx()).await.unwrap();
        let got = c.get(w.id, &ctx()).await.unwrap();
        assert_eq!(got.id, w.id);
        assert_eq!(got.data.get("active").and_then(|v| v.as_bool()), Some(true));
    }

    #[tokio::test]
    async fn stub_get_unknown_returns_not_found() {
        let c = StubWorkerClient::new();
        let err = c.get(Uuid::new_v4(), &ctx()).await.unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[tokio::test]
    async fn stub_delete_then_get_404s() {
        let c = StubWorkerClient::new();
        let w = c.create(make_worker_create(), &ctx()).await.unwrap();
        c.delete(w.id, &ctx()).await.unwrap();
        assert!(c.is_empty());
        let err = c.get(w.id, &ctx()).await.unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[tokio::test]
    async fn stub_merge_drops_source() {
        let c = StubWorkerClient::new();
        let a = c.create(make_worker_create(), &ctx()).await.unwrap();
        let b = c.create(make_worker_create(), &ctx()).await.unwrap();
        let out = c.merge(a.id, b.id, &ctx()).await.unwrap();
        assert_eq!(out.source_id, a.id);
        assert_eq!(out.target_id, b.id);
        assert!(matches!(
            c.get(a.id, &ctx()).await.unwrap_err(),
            Error::NotFound(_)
        ));
    }

    // ─── HTTP impl tests ──────────────────────────────────────────────

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

    fn leak(bytes: Vec<u8>) -> &'static [u8] {
        Box::leak(bytes.into_boxed_slice())
    }

    #[tokio::test]
    async fn http_get_unwraps_envelope() {
        let id = Uuid::new_v4();
        let body =
            format!(r#"{{"success":true,"data":{{"id":"{id}","active":true}},"error":null}}"#);
        let resp = leak(http_envelope_ok(&body));
        let addr = one_shot_server(resp).await;
        let client =
            HttpWorkerClient::new(format!("http://{addr}"), Duration::from_secs(2), None).unwrap();
        let w = client.get(id, &ctx()).await.unwrap();
        assert_eq!(w.id, id);
    }

    #[tokio::test]
    async fn http_get_404_envelope_becomes_not_found() {
        let body =
            r#"{"success":false,"data":null,"error":{"code":"NOT_FOUND","message":"missing"}}"#;
        let resp = leak(http_envelope_status("404 Not Found", body));
        let addr = one_shot_server(resp).await;
        let client =
            HttpWorkerClient::new(format!("http://{addr}"), Duration::from_secs(2), None).unwrap();
        let err = client.get(Uuid::new_v4(), &ctx()).await.unwrap_err();
        assert!(matches!(err, Error::NotFound(_)));
    }

    #[tokio::test]
    async fn http_transport_failure_becomes_upstream() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        let client =
            HttpWorkerClient::new(format!("http://{addr}"), Duration::from_millis(500), None)
                .unwrap();
        let err = client.get(Uuid::new_v4(), &ctx()).await.unwrap_err();
        assert!(matches!(err, Error::Upstream(_)));
    }
}
