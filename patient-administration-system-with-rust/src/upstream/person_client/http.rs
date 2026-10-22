//! Part of [`super`]: http.

use super::*;

/// reqwest-backed [`PersonClient`] that speaks the upstream REST API.
pub struct HttpPersonClient {
    http: reqwest::Client,
    base_url: String,
    bearer: Option<String>,
}

impl HttpPersonClient {
    /// Build a new HTTP client.
    ///
    /// `base_url` is the upstream's root (e.g. `http://localhost:5180`),
    /// without trailing slash. `timeout` applies to the whole request.
    /// `bearer` is attached as `Authorization: Bearer <token>` when `Some`.
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

    pub(super) fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Apply common headers (bearer auth + pass-through user identity) to
    /// every outbound request.
    pub(super) fn enrich(
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

/// PAS-style response envelope returned by the upstream.
#[derive(Debug, Deserialize)]
pub(super) struct UpstreamEnvelope<T> {
    success: bool,
    data: Option<T>,
    error: Option<UpstreamError>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpstreamError {
    code: String,
    message: String,
}

/// Translate an upstream error envelope to a typed [`crate::Error`].
pub(super) fn translate_envelope_error(status: reqwest::StatusCode, err: UpstreamError) -> Error {
    let UpstreamError { code, message } = err;
    let msg = format!("upstream {code}: {message}");
    match status.as_u16() {
        404 => Error::NotFound(msg),
        400 | 422 => Error::Validation(msg),
        409 => Error::Conflict(msg),
        _ => Error::Upstream(msg),
    }
}

/// Send a request expecting a JSON envelope wrapping `T`.
pub(super) async fn send_envelope<T: serde::de::DeserializeOwned>(
    rb: reqwest::RequestBuilder,
) -> Result<T> {
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

/// Same as `send_envelope` but for endpoints whose `data` is `null` /
/// absent on success (e.g. `DELETE`).
pub(super) async fn send_envelope_unit(rb: reqwest::RequestBuilder) -> Result<()> {
    let resp = rb
        .send()
        .await
        .map_err(|e| Error::upstream(format!("transport: {e}")))?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    // On non-2xx, attempt envelope parse for a typed error.
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
impl PersonClient for HttpPersonClient {
    async fn create(&self, person: PersonCreate, ctx: &UserContext) -> Result<Person> {
        let rb = self.enrich(self.http.post(self.url("/api/persons")).json(&person), ctx);
        send_envelope(rb).await
    }

    async fn get(&self, id: Uuid, ctx: &UserContext) -> Result<Person> {
        let rb = self.enrich(self.http.get(self.url(&format!("/api/persons/{id}"))), ctx);
        send_envelope(rb).await
    }

    async fn update(&self, id: Uuid, person: PersonCreate, ctx: &UserContext) -> Result<Person> {
        let rb = self.enrich(
            self.http
                .put(self.url(&format!("/api/persons/{id}")))
                .json(&person),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn delete(&self, id: Uuid, ctx: &UserContext) -> Result<()> {
        let rb = self.enrich(
            self.http.delete(self.url(&format!("/api/persons/{id}"))),
            ctx,
        );
        send_envelope_unit(rb).await
    }

    async fn search(&self, query: &str, limit: u32, ctx: &UserContext) -> Result<Vec<Person>> {
        let rb = self.enrich(
            self.http
                .get(self.url("/api/persons/search"))
                .query(&[("q", query), ("limit", &limit.to_string())]),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn match_persons(
        &self,
        candidate: PersonCreate,
        ctx: &UserContext,
    ) -> Result<Vec<PersonMatch>> {
        let rb = self.enrich(
            self.http
                .post(self.url("/api/persons/match"))
                .json(&candidate),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn check_duplicates(
        &self,
        candidate: PersonCreate,
        ctx: &UserContext,
    ) -> Result<Vec<PersonMatch>> {
        let rb = self.enrich(
            self.http
                .post(self.url("/api/persons/check-duplicates"))
                .json(&candidate),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn merge(&self, source: Uuid, target: Uuid, ctx: &UserContext) -> Result<MergeOutcome> {
        let body = serde_json::json!({ "source_id": source, "target_id": target });
        let rb = self.enrich(
            self.http.post(self.url("/api/persons/merge")).json(&body),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn masked(&self, id: Uuid, ctx: &UserContext) -> Result<Person> {
        let rb = self.enrich(
            self.http
                .get(self.url(&format!("/api/persons/{id}/masked"))),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn export(&self, id: Uuid, ctx: &UserContext) -> Result<serde_json::Value> {
        let rb = self.enrich(
            self.http
                .get(self.url(&format!("/api/persons/{id}/export"))),
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
                .get(self.url(&format!("/api/persons/{id}/audit")))
                .query(&[("limit", &limit.to_string())]),
            ctx,
        );
        send_envelope(rb).await
    }

    async fn bulk_export(&self, format: &str, ctx: &UserContext) -> Result<(String, Vec<u8>)> {
        if !matches!(format, "json" | "xml" | "tsv" | "csv") {
            return Err(Error::validation(format!(
                "bulk_export: unsupported format {format:?} (expected json/xml/tsv/csv)"
            )));
        }
        let rb = self.enrich(
            self.http
                .get(self.url(&format!("/api/persons/export.{format}"))),
            ctx,
        );
        let resp = rb
            .send()
            .await
            .map_err(|e| Error::upstream(format!("transport: {e}")))?;
        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let body = resp
            .bytes()
            .await
            .map_err(|e| Error::upstream(format!("read body: {e}")))?
            .to_vec();
        if !status.is_success() {
            return Err(Error::upstream(format!(
                "bulk_export upstream returned {status}: {} bytes",
                body.len()
            )));
        }
        Ok((content_type, body))
    }

    async fn bulk_import(
        &self,
        content_type: &str,
        body: Vec<u8>,
        ctx: &UserContext,
    ) -> Result<serde_json::Value> {
        let rb = self.enrich(
            self.http
                .post(self.url("/api/persons/import"))
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .body(body),
            ctx,
        );
        send_envelope(rb).await
    }
}
