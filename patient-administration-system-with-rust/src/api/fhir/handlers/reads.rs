//! Part of [`super`]: reads.

use super::*;

pub(super) fn wrap_fhir(resource_type: &str, mut value: Value) -> Value {
    if let Value::Object(ref mut map) = value {
        map.insert(
            "resourceType".to_string(),
            Value::String(resource_type.to_string()),
        );
    }
    value
}

#[utoipa::path(
    get,
    path = "/fhir/Encounter/{id}",
    tag = "FHIR",
    summary = "Get encounter",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_encounter(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult {
    match EncounterRepository::find_by_id(&state.db, id).await {
        Ok(Some(e)) => Ok(Json(crate::api::fhir::r5::encounter(&e))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(operation_outcome::not_found(format!("Encounter/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

#[utoipa::path(
    get,
    path = "/fhir/Appointment/{id}",
    tag = "FHIR",
    summary = "Get appointment",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_appointment(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult {
    match AppointmentRepository::find_by_id(&state.db, id).await {
        Ok(Some(a)) => Ok(Json(crate::api::fhir::r5::appointment(&a))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(operation_outcome::not_found(format!("Appointment/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

/// One bound of `_lastUpdated`: `ge`, `gt`, `lt` or `le` and an instant (or a date, its midnight UTC).
fn last_updated_bound(v: &str) -> Result<(&str, jiff::Timestamp), Error> {
    let bad = || {
        Error::validation(format!(
            "_lastUpdated {v:?}: give ge, gt, lt or le and an instant, such as ge2026-11-01T00:00:00Z"
        ))
    };
    let (prefix, rest) = (v.get(..2).ok_or_else(bad)?, v.get(2..).ok_or_else(bad)?);
    if !matches!(prefix, "ge" | "gt" | "lt" | "le") {
        return Err(bad());
    }
    let at = match rest.parse::<jiff::Timestamp>() {
        Ok(t) => t,
        Err(_) => rest
            .parse::<jiff::civil::Date>()
            .map_err(|_| bad())?
            .to_zoned_datetime(jiff::tz::TimeZone::UTC)
            .map_err(|_| bad())?
            .timestamp(),
    };
    Ok((prefix, at))
}

/// `_lastUpdated` as the half-open window `[since, before)` the repository reads (TSC-6).
pub(crate) fn last_updated_window(
    values: &[&str],
) -> Result<(jiff::Timestamp, Option<jiff::Timestamp>), Error> {
    let tick = jiff::SignedDuration::from_micros(1);
    let (mut since, mut before) = (None, None);
    for v in values {
        let (prefix, at) = last_updated_bound(v)?;
        let slot = if matches!(prefix, "ge" | "gt") {
            &mut since
        } else {
            &mut before
        };
        if slot.is_some() {
            return Err(Error::validation(
                "_lastUpdated takes one lower and one upper bound at most",
            ));
        }
        // Postgres keeps microseconds, so the next microsecond is the first instant after `at`.
        *slot = Some(if matches!(prefix, "gt" | "le") {
            at.checked_add(tick)
                .map_err(|e| Error::validation(format!("_lastUpdated: {e}")))?
        } else {
            at
        });
    }
    let since = since.ok_or_else(|| {
        Error::validation("search by _lastUpdated: a lower bound (ge or gt) is required")
    })?;
    if before.is_some_and(|b| b <= since) {
        return Err(Error::validation(
            "_lastUpdated: the upper bound must be after the lower",
        ));
    }
    Ok((since, before))
}

#[utoipa::path(
    get,
    path = "/fhir/Appointment",
    tag = "FHIR",
    summary = "Search appointments by when they last changed",
    params(
        ("_lastUpdated" = String, Query, description = "Changed in this window: `ge` or `gt` an instant (required), and optionally `lt` or `le` one; repeat the parameter for both"),
        ("_count" = Option<i64>, Query, description = "Entries per page, 1 to 500 (default 100); a `next` link gives the rest"),
        ("_offset" = Option<i64>, Query, description = "Where the page starts; set by the `next` link"),
    ),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
/// `GET /fhir/Appointment?_lastUpdated=ge…` (TSC-6): appointments changed in a window, oldest change first,
/// for a system that polls. A deleted appointment is in the list as `entered-in-error`, so the poller learns
/// of it. Walk a fixed window (give an upper bound) and use it as the next poll's lower bound.
pub async fn search_appointments(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<Vec<(String, String)>>,
) -> FhirResult {
    let values = |name: &str| -> Vec<&str> {
        q.iter()
            .filter(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
            .collect()
    };
    if let Some((k, _)) = q
        .iter()
        .find(|(k, _)| !matches!(k.as_str(), "_lastUpdated" | "_count" | "_offset"))
    {
        return Err(error_to_response(Error::validation(format!(
            "Appointment search does not support {k}; it takes _lastUpdated, _count and _offset"
        ))));
    }
    let (since, before) =
        last_updated_window(&values("_lastUpdated")).map_err(error_to_response)?;
    let number = |name: &str, default: i64, min: i64, max: i64| -> Result<i64, Error> {
        match values(name).as_slice() {
            [] => Ok(default),
            [v] => v
                .parse::<i64>()
                .ok()
                .filter(|n| (min..=max).contains(n))
                .ok_or_else(|| {
                    Error::validation(format!("{name} must be a whole number from {min} to {max}"))
                }),
            _ => Err(Error::validation(format!("{name} is given once"))),
        }
    };
    let count = number("_count", 100, 1, 500).map_err(error_to_response)?;
    let offset = number("_offset", 0, 0, i64::MAX).map_err(error_to_response)?;
    let fetch = count
        .checked_add(1)
        .ok_or_else(|| error_to_response(Error::validation("_count")))?;
    let mut rows = AppointmentRepository::list_changed(
        &state.db,
        since,
        before,
        None,
        None,
        crate::paging::Rows::window(fetch, offset),
    )
    .await
    .map_err(error_to_response)?;
    let more = i64::try_from(rows.len()).unwrap_or(i64::MAX) > count;
    rows.truncate(usize::try_from(count).unwrap_or(usize::MAX));
    let resources: Vec<Value> = rows
        .iter()
        .map(|a| {
            let mut r = crate::api::fhir::r5::appointment(a);
            if let Value::Object(m) = &mut r {
                m.insert(
                    "meta".into(),
                    serde_json::json!({ "lastUpdated": a.updated_at }),
                );
                if a.deleted_at.is_some() {
                    m.insert("status".into(), Value::String("entered-in-error".into()));
                }
            }
            r
        })
        .collect();
    let mut query = format!("_lastUpdated=ge{since}");
    if let Some(b) = before {
        query.push_str(&format!("&_lastUpdated=lt{b}"));
    }
    query.push_str(&format!("&_count={count}"));
    let base = crate::api::fhir::r5::base_url(&headers);
    let mut bundle = crate::api::fhir::r5::searchset(
        &base,
        &format!("/Appointment?{query}&_offset={offset}").replace('+', "%2B"),
        resources,
    );
    if let Value::Object(m) = &mut bundle {
        // The page's size is not the number of matches, so no total is claimed.
        m.remove("total");
        if more && let Some(Value::Array(links)) = m.get_mut("link") {
            let next = offset.checked_add(count).unwrap_or(i64::MAX);
            links.push(serde_json::json!({
                "relation": "next",
                "url": format!("{base}/Appointment?{query}&_offset={next}").replace('+', "%2B"),
            }));
        }
    }
    Ok(Json(bundle))
}

#[utoipa::path(
    get,
    path = "/fhir/Coverage/{id}",
    tag = "FHIR",
    summary = "Get coverage",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_coverage(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult {
    match CoverageRepository::find_by_id(&state.db, id).await {
        Ok(Some(c)) => Ok(Json(crate::api::fhir::r5::coverage(&c))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(operation_outcome::not_found(format!("Coverage/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

#[utoipa::path(
    get,
    path = "/fhir/Schedule/{id}",
    tag = "FHIR",
    summary = "Get schedule",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_schedule(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult {
    match ScheduleRepository::find_by_id(&state.db, id).await {
        Ok(Some(s)) => Ok(Json(crate::api::fhir::r5::schedule(&s))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(operation_outcome::not_found(format!("Schedule/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

#[utoipa::path(
    get,
    path = "/fhir/Slot/{id}",
    tag = "FHIR",
    summary = "Get slot",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_slot(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult {
    match SlotRepository::find_by_id(&state.db, id).await {
        Ok(Some(s)) => Ok(Json(crate::api::fhir::r5::slot(&s))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(operation_outcome::not_found(format!("Slot/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

/// FHIR Location read backed by the PAS `Bed` row.
#[utoipa::path(
    get,
    path = "/fhir/Location/{id}",
    tag = "FHIR",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn get_location(State(state): State<AppState>, Path(id): Path<Uuid>) -> FhirResult {
    match BedRepository::find_by_id(&state.db, id).await {
        Ok(Some(b)) => Ok(Json(crate::api::fhir::r5::location(&b))),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(operation_outcome::not_found(format!("Location/{id}"))),
        )),
        Err(e) => Err(error_to_response(e)),
    }
}

#[utoipa::path(
    get,
    path = "/fhir/Flag",
    tag = "FHIR",
    summary = "Search flags (reasonable adjustments)",
    params(("patient" = String, Query, description = "The patient, as `Patient/<id>` or the bare id (required)")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn search_flags(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> FhirResult {
    let patient = q
        .get("patient")
        .map(|p| p.strip_prefix("Patient/").unwrap_or(p))
        .and_then(|p| Uuid::parse_str(p).ok())
        .ok_or_else(|| {
            error_to_response(Error::validation(
                "search by patient: ?patient=Patient/<id> is required",
            ))
        })?;
    let status = q.get("status").map(String::as_str);
    if !matches!(status, None | Some("active" | "inactive")) {
        return Err(error_to_response(Error::validation(
            "status must be active or inactive",
        )));
    }
    let all = crate::adjustments::AdjustmentService::new(state.db.clone())
        .for_patient(patient, status != Some("active"), 201)
        .await
        .map_err(error_to_response)?;
    let flags: Vec<Value> = all
        .iter()
        .filter(|a| status != Some("inactive") || a.ended_at.is_some())
        .map(|a| a.fhir.clone())
        .collect();
    let mut self_url = format!("/Flag?patient=Patient/{patient}");
    if let Some(s) = status {
        self_url.push_str(&format!("&status={s}"));
    }
    Ok(Json(crate::api::fhir::r5::searchset(
        &crate::api::fhir::r5::base_url(&headers),
        &self_url,
        flags,
    )))
}

#[utoipa::path(
    get,
    path = "/fhir/Observation",
    tag = "FHIR",
    summary = "Search triage categories as Observations",
    params(("encounter" = String, Query, description = "The encounter, as `Encounter/<id>` or the bare id (required)")),
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn search_observations(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> FhirResult {
    let encounter = q
        .get("encounter")
        .map(|p| p.strip_prefix("Encounter/").unwrap_or(p))
        .and_then(|p| Uuid::parse_str(p).ok())
        .ok_or_else(|| {
            error_to_response(Error::validation(
                "search by encounter: ?encounter=Encounter/<id> is required",
            ))
        })?;
    let all = crate::triage::TriageService::new(state.db.clone())
        .observations(encounter)
        .await
        .map_err(error_to_response)?;
    Ok(Json(crate::api::fhir::r5::searchset(
        &crate::api::fhir::r5::base_url(&headers),
        &format!("/Observation?encounter=Encounter/{encounter}"),
        all,
    )))
}

#[utoipa::path(
    get,
    path = "/fhir/SubscriptionTopic",
    tag = "FHIR",
    summary = "Search subscription topics",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn search_subscription_topics() -> FhirResult {
    let entries: Vec<Value> = crate::subscriptions::TOPICS
        .iter()
        .map(|(name, url)| {
            serde_json::json!({ "resource": {
                "resourceType": "SubscriptionTopic",
                "id": name,
                "url": url,
                "status": "active",
                "title": format!("Changes to {name}"),
            } })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "resourceType": "Bundle", "type": "searchset", "total": entries.len(), "entry": entries,
    })))
}

#[utoipa::path(
    get,
    path = "/fhir/Subscription",
    tag = "FHIR",
    summary = "Search subscriptions",
    responses(
        (status = 200, description = "FHIR resource, Bundle or OperationOutcome"),
        (status = "4XX", description = "Error; an OperationOutcome"),
        (status = "5XX", description = "Server or upstream error; an OperationOutcome"),
    )
)]
pub async fn search_subscriptions(State(state): State<AppState>) -> FhirResult {
    let all = crate::subscriptions::SubscriptionService::new(state.db.clone())
        .list()
        .await
        .map_err(error_to_response)?;
    let entries: Vec<Value> = all
        .iter()
        .map(|s| {
            serde_json::json!({ "resource": {
                "resourceType": "Subscription",
                "id": s.id,
                "status": if s.status == "active" { "active" } else { "off" },
                "topic": crate::subscriptions::topic_url(&s.topic),
                "channelType": {
                    "system": "http://terminology.hl7.org/CodeSystem/subscription-channel-type",
                    "code": "rest-hook",
                },
                "endpoint": s.endpoint,
                "contentType": "application/fhir+json",
                "content": "id-only",
            } })
        })
        .collect();
    Ok(Json(serde_json::json!({
        "resourceType": "Bundle", "type": "searchset", "total": entries.len(), "entry": entries,
    })))
}

#[cfg(test)]
mod last_updated_tests {
    use super::last_updated_window;

    #[test]
    fn the_window_is_half_open_and_needs_a_lower_bound() {
        let (s, b) = last_updated_window(&["ge2026-11-01T00:00:00Z", "lt2026-11-02"]).unwrap();
        assert_eq!(s.to_string(), "2026-11-01T00:00:00Z");
        assert_eq!(b.unwrap().to_string(), "2026-11-02T00:00:00Z");
        let (s, b) =
            last_updated_window(&["gt2026-11-01T00:00:00Z", "le2026-11-01T00:00:01Z"]).unwrap();
        assert_eq!(s.to_string(), "2026-11-01T00:00:00.000001Z");
        assert_eq!(b.unwrap().to_string(), "2026-11-01T00:00:01.000001Z");
        for bad in [
            &["lt2026-11-02"][..],
            &["eq2026-11-02"],
            &["ge2026-11-02", "ge2026-11-03"],
            &["ge2026-11-02", "lt2026-11-01"],
            &["genot-a-date"],
            &[""],
        ] {
            assert!(last_updated_window(bad).is_err(), "{bad:?}");
        }
    }
}
