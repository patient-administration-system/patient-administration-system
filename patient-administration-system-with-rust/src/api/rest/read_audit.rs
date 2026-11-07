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
//! names, dates and ids. The context also says how many records came back (`returned`), read from the body:
//! the envelope's `meta.returned` or `data`, a FHIR Bundle's `entry`, or a bare array.
//!
//! **Lists and exports of people (OFH-2).** Any other successful response that lists people says so with a
//! [`PeopleListed`] response extension, and writes the same `patient_search` row with its count. The list
//! helpers set it on every list whose rows carry a `person_id` (`handlers::ok_json_page`), so a new list is
//! covered without being named here; the patient exports set it with the records they hold, or no count
//! for XML, whose shape is the person-service's. A list of rows with no `person_id`, or a report of counts,
//! writes nothing. The retention is in `governance/retention-schedule.md`.

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

/// A response that lists people, and how many when that is known (OFH-2). The audit middleware writes one
/// `patient_search` row for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeopleListed(pub Option<usize>);

/// `Some(n)` when `rows` is an array of `n` rows of which at least one names a person (`person_id`).
pub fn people_in(rows: &serde_json::Value) -> Option<usize> {
    let rows = rows.as_array()?;
    rows.iter()
        .any(|r| r.get("person_id").is_some_and(|v| !v.is_null()))
        .then_some(rows.len())
}

/// How many records a search answered with: the envelope's `meta.returned` or `data`, a FHIR Bundle's
/// `entry`, or a bare array. `None` when the body is none of these.
pub fn records_in(body: &[u8]) -> Option<usize> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    if let Some(n) = v["meta"]["returned"].as_u64() {
        return usize::try_from(n).ok();
    }
    if let Some(rows) = v["data"].as_array() {
        return Some(rows.len());
    }
    if v["resourceType"] == "Bundle" {
        return Some(v["entry"].as_array().map_or(0, Vec::len));
    }
    v.as_array().map(Vec::len)
}

/// The largest search body read to count its records. A bigger one is recorded with no count.
const MAX_COUNTED_BODY: usize = 32 * 1024 * 1024;

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
    Some((path.to_string(), query_names(query)))
}

/// The sorted, de-duplicated names of a query's parameters, never their values.
fn query_names(query: Option<&str>) -> Vec<String> {
    let mut names: Vec<String> = query
        .unwrap_or_default()
        .split('&')
        .filter_map(|pair| pair.split('=').next())
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    names
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
    let path = req.uri().path().to_string();
    let names = query_names(req.uri().query());
    let res = next.run(req).await;
    if !res.status().is_success() {
        return res;
    }
    let status = res.status().as_u16();
    let listed = res.extensions().get::<PeopleListed>().copied();
    let (res, entity_type, entity_id, context) = match (target, search, listed) {
        (Some((patient, route)), _, _) => (
            res,
            "patient",
            patient,
            serde_json::json!({ "route": route, "status": status }),
        ),
        (None, Some((route, params)), _) => {
            // The count is in the body, so read it and put it back.
            let (parts, body) = res.into_parts();
            let (res, returned) = match axum::body::to_bytes(body, MAX_COUNTED_BODY).await {
                Ok(bytes) => {
                    let returned = records_in(&bytes);
                    (Response::from_parts(parts, Body::from(bytes)), returned)
                }
                Err(e) => {
                    tracing::error!(error = %e, "a search body could not be read to count it");
                    return error_to_response(Error::internal("the read could not be recorded"))
                        .into_response();
                }
            };
            (
                res,
                "patient_search",
                Uuid::nil(),
                serde_json::json!({
                    "route": route,
                    "status": status,
                    "query_params": params,
                    "returned": returned,
                }),
            )
        }
        (None, None, Some(PeopleListed(returned))) => (
            res,
            "patient_search",
            Uuid::nil(),
            serde_json::json!({
                "route": path,
                "status": status,
                "query_params": names,
                "returned": returned,
            }),
        ),
        (None, None, None) => return res,
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
    fn a_list_names_people_only_when_its_rows_carry_a_person_id() {
        use serde_json::json;
        assert_eq!(
            people_in(&json!([{ "person_id": ID }, { "person_id": null }])),
            Some(2)
        );
        assert_eq!(
            people_in(&json!([{ "id": ID, "name": "a template" }])),
            None
        );
        assert_eq!(people_in(&json!([{ "person_id": null }])), None);
        assert_eq!(people_in(&json!([])), None);
        assert_eq!(people_in(&json!({ "person_id": ID })), None);
    }

    #[test]
    fn a_search_is_counted_from_its_body() {
        let n = |b: &str| records_in(b.as_bytes());
        assert_eq!(n(r#"{"data":[1,2,3],"meta":{"returned":2}}"#), Some(2));
        assert_eq!(n(r#"{"success":true,"data":[1,2,3]}"#), Some(3));
        assert_eq!(n(r#"{"resourceType":"Bundle","entry":[{},{}]}"#), Some(2));
        assert_eq!(n(r#"{"resourceType":"Bundle","total":0}"#), Some(0));
        assert_eq!(n("[1]"), Some(1));
        assert_eq!(n("<xml/>"), None);
        assert_eq!(n(r#"{"data":{"one":1}}"#), None);
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
