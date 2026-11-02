//! Who read a patient's records (NEW-9). Until now only changes were audited.
//!
//! A successful `GET` of `/api/patients/{id}` or anything under it, or of `/fhir/Patient/{id}`, writes one
//! `read` row to `audit_log` for that patient, with the route (never the query string, which can carry
//! names and dates) and the status in `context`. A failed read writes nothing. If the row cannot be written
//! the read is refused rather than served unrecorded.
//!
//! **Searches and batch reads (OFH-2).** A successful `GET` of `/api/patients/search`, `/api/patients`,
//! `/fhir/Patient`, `/fhir/Flag` or `/api/reasonable-adjustments/for-appointments` writes one `read` row of
//! entity type `patient_search` (entity id the nil UUID, because no one patient is named) whose context is
//! the route, the status and the *names* of the query parameters, sorted. Never the values, which carry
//! names, dates and ids. **Not covered:** how many records came back (that needs the body), the report and
//! export routes, and any other list route. The retention is in `governance/retention-schedule.md`.

use axum::{
    body::Body,
    extract::{Request, State},
    http::Method,
    middleware::Next,
    response::{IntoResponse, Response},
};
use uuid::Uuid;

use crate::Error;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{error_to_response, request_audit_ctx};
use crate::db::repositories::audit::AuditLogRepository;

/// The patient a read names and the route with the id replaced by `{id}`, or `None` when the path is not a
/// single patient's. Pure, so the rule is tested without a database.
pub fn patient_read_target(method: &Method, path: &str) -> Option<(Uuid, String)> {
    if method != Method::GET && method != Method::HEAD {
        return None;
    }
    let rest = path
        .strip_prefix("/api/patients/")
        .or_else(|| path.strip_prefix("/fhir/Patient/"))?;
    let (id, tail) = match rest.split_once('/') {
        Some((id, tail)) => (id, Some(tail)),
        None => (rest, None),
    };
    let id = Uuid::parse_str(id).ok()?;
    let prefix = &path[..path.len() - rest.len()];
    Some((
        id,
        match tail {
            Some(t) => format!("{prefix}{{id}}/{t}"),
            None => format!("{prefix}{{id}}"),
        },
    ))
}

/// The routes that read many patients' data at once, by exact path (OFH-2).
const SEARCH_ROUTES: [&str; 5] = [
    "/api/patients/search",
    "/api/patients",
    "/fhir/Patient",
    "/fhir/Flag",
    "/api/reasonable-adjustments/for-appointments",
];

/// For a search or batch read: the route and the sorted, de-duplicated names of its query parameters
/// (never the values). `None` for any other request. Pure, so it is tested without a database.
pub fn search_read_target(
    method: &Method,
    path: &str,
    query: Option<&str>,
) -> Option<(String, Vec<String>)> {
    if method != Method::GET && method != Method::HEAD {
        return None;
    }
    if !SEARCH_ROUTES.contains(&path) {
        return None;
    }
    let mut names: Vec<String> = query
        .unwrap_or_default()
        .split('&')
        .filter_map(|pair| pair.split('=').next())
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    Some((path.to_string(), names))
}

/// Record the read when it succeeded; refuse the response if the record cannot be written.
pub async fn read_audit_middleware(
    State(state): State<AppState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let target = patient_read_target(req.method(), req.uri().path());
    let search = search_read_target(req.method(), req.uri().path(), req.uri().query());
    let ctx = request_audit_ctx(req.headers());
    let res = next.run(req).await;
    if !res.status().is_success() {
        return res;
    }
    let (entity_type, entity_id, context) = match (target, search) {
        (Some((patient, route)), _) => (
            "patient",
            patient,
            serde_json::json!({ "route": route, "status": res.status().as_u16() }),
        ),
        (None, Some((route, params))) => (
            "patient_search",
            Uuid::nil(),
            serde_json::json!({
                "route": route,
                "status": res.status().as_u16(),
                "query_params": params,
            }),
        ),
        (None, None) => return res,
    };
    let logged = AuditLogRepository::log_with(
        &state.db,
        entity_type,
        entity_id,
        "read",
        None,
        None,
        None,
        Some(context),
        &ctx,
    )
    .await;
    match logged {
        Ok(()) => res,
        Err(e) => {
            tracing::error!(error = %e, "read audit row could not be written; read refused");
            error_to_response(Error::internal("the read could not be recorded")).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0b5a3c1e-2f4d-4a6b-8c7d-9e0f1a2b3c4d";

    #[test]
    fn a_get_under_one_patient_is_a_read_of_that_patient() {
        let t = |p: &str| patient_read_target(&Method::GET, p);
        let (id, route) = t(&format!("/api/patients/{ID}/appointments")).unwrap();
        assert_eq!(id.to_string(), ID);
        assert_eq!(route, "/api/patients/{id}/appointments");
        assert_eq!(
            t(&format!("/api/patients/{ID}")).unwrap().1,
            "/api/patients/{id}"
        );
        assert_eq!(
            t(&format!("/fhir/Patient/{ID}")).unwrap().1,
            "/fhir/Patient/{id}"
        );
    }

    #[test]
    fn a_search_records_the_parameter_names_and_never_the_values() {
        let t = |p: &str, q: Option<&str>| search_read_target(&Method::GET, p, q);
        let (route, names) = t(
            "/api/patients/search",
            Some("family=Jones&dob=1980-01-02&family=Smith&"),
        )
        .unwrap();
        assert_eq!(route, "/api/patients/search");
        assert_eq!(names, vec!["dob", "family"]);
        assert!(!format!("{names:?}").contains("Jones"));
        assert_eq!(t("/fhir/Flag", None).unwrap().1, Vec::<String>::new());
        assert!(t("/api/patients/export.json", None).is_none());
        assert!(t(&format!("/api/patients/{ID}"), None).is_none());
        assert!(search_read_target(&Method::POST, "/api/patients/search", None).is_none());
    }

    #[test]
    fn searches_lists_writes_and_other_paths_are_not() {
        let get = |p: &str| patient_read_target(&Method::GET, p);
        assert!(get("/api/patients/search").is_none());
        assert!(get("/api/patients").is_none());
        assert!(get("/api/patients/export.json").is_none());
        assert!(get("/fhir/Patient").is_none());
        assert!(get(&format!("/api/practitioners/{ID}")).is_none());
        assert!(patient_read_target(&Method::POST, &format!("/api/patients/{ID}/pifu")).is_none());
        assert!(patient_read_target(&Method::PUT, &format!("/fhir/Patient/{ID}")).is_none());
    }
}
