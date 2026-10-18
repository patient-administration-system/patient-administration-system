//! Thin reqwest client wrapping the upstream `person-service` REST API.
//!
//! v0.2.0 frontend reads (patient list + detail pages, plus the
//! patient picker inputs on the admissions / appointments / letters
//! pages) no longer hit the local `patients` table (which was dropped
//! in PAS v0.44.0 Option C identity delegation). They proxy upstream
//! to `person-service` (`:5180` in dev) so identity stays single-sourced.

use std::sync::OnceLock;
use std::time::Duration;

use serde::Deserialize;
use uuid::Uuid;

const DEFAULT_BASE_URL: &str = "http://localhost:5180";
const DEFAULT_TIMEOUT_SECS: u64 = 10;

/// Base URL of the upstream person-service. Configurable via
/// `PERSON_SERVICE_URL` (matches the env var the PAS Axum binary uses
/// so a single-machine dev deploy can share one value).
pub fn base_url() -> String {
    std::env::var("PERSON_SERVICE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_string())
}

fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .build()
            .expect("reqwest client build")
    })
}

#[derive(Debug, thiserror::Error)]
pub enum PersonApiError {
    #[error("person-service HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("person-service returned status {status}")]
    Status { status: u16 },
    #[error("person-service response missing expected `data` field")]
    MalformedResponse,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    success: bool,
    data: Option<T>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct UpstreamPerson {
    pub id: Uuid,
    #[serde(default)]
    pub active: Option<bool>,
    #[serde(default)]
    pub name: Option<serde_json::Value>,
    #[serde(default)]
    pub names: Option<serde_json::Value>,
    #[serde(default)]
    pub gender: Option<String>,
    #[serde(default)]
    pub birth_date: Option<String>,
    #[serde(default)]
    pub deceased: Option<bool>,
    #[serde(default)]
    pub marital_status: Option<String>,
    #[serde(default)]
    pub identifiers: Option<serde_json::Value>,
    #[serde(default)]
    pub telecom: Option<serde_json::Value>,
    #[serde(default)]
    pub addresses: Option<serde_json::Value>,
}

/// `GET /api/persons?limit=N` — list active persons.
pub async fn list_persons(limit: u32) -> Result<Vec<UpstreamPerson>, PersonApiError> {
    let url = format!("{}/api/persons", base_url());
    let resp = client()
        .get(&url)
        .query(&[("limit", limit.to_string())])
        .send()
        .await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(PersonApiError::Status {
            status: status.as_u16(),
        });
    }
    // person-service may return either `{ success, data: [...] }` (PAS
    // envelope) or a bare array; tolerate both.
    let body = resp.text().await?;
    if let Ok(env) = serde_json::from_str::<Envelope<Vec<UpstreamPerson>>>(&body)
        && env.success
        && let Some(d) = env.data
    {
        return Ok(d);
    }
    let arr: Vec<UpstreamPerson> =
        serde_json::from_str(&body).map_err(|_| PersonApiError::MalformedResponse)?;
    Ok(arr)
}

/// `GET /api/persons/{id}` — fetch one person.
pub async fn get_person(id: Uuid) -> Result<Option<UpstreamPerson>, PersonApiError> {
    let url = format!("{}/api/persons/{id}", base_url());
    let resp = client().get(&url).send().await?;
    let status = resp.status();
    if status.as_u16() == 404 {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(PersonApiError::Status {
            status: status.as_u16(),
        });
    }
    let body = resp.text().await?;
    if let Ok(env) = serde_json::from_str::<Envelope<UpstreamPerson>>(&body)
        && env.success
        && let Some(d) = env.data
    {
        return Ok(Some(d));
    }
    let p: UpstreamPerson =
        serde_json::from_str(&body).map_err(|_| PersonApiError::MalformedResponse)?;
    Ok(Some(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_defaults_when_env_unset() {
        // Best-effort: only assert when the env happens to be unset to
        // keep the test deterministic in dev shells that *do* set it.
        if std::env::var("PERSON_SERVICE_URL").is_err() {
            assert_eq!(base_url(), DEFAULT_BASE_URL);
        }
    }

    #[test]
    fn upstream_person_deserializes_minimal_object() {
        let v = serde_json::json!({ "id": Uuid::new_v4().to_string() });
        let p: UpstreamPerson = serde_json::from_value(v).expect("parse");
        assert!(p.active.is_none());
        assert!(p.identifiers.is_none());
    }

    #[test]
    fn upstream_person_deserializes_full_object() {
        let v = serde_json::json!({
            "id": Uuid::new_v4().to_string(),
            "active": true,
            "name": { "family": "Doe", "given": ["Jane"] },
            "gender": "female",
            "birth_date": "1990-01-15",
            "identifiers": [{ "value": "MRN-99" }]
        });
        let p: UpstreamPerson = serde_json::from_value(v).expect("parse");
        assert_eq!(p.active, Some(true));
        assert_eq!(p.gender.as_deref(), Some("female"));
    }
}
