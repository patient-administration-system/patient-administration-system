//! Part of [`super`]: adt changes.

use super::*;

#[utoipa::path(
    post,
    path = "/api/hl7/v2/pre-admit",
    tag = "Hl7",
    summary = "Pre admit",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn pre_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let bed_code = match r.message.segment("PV1").and_then(extract_bed_code_from_pv1) {
        Some(c) => c,
        None => return ae(&r.sender, &r.control_id, "PV1-3.3 (bed) is required"),
    };
    let bed = match BedRepository::find_by_code(&state.db, &bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("bed code {bed_code:?} not found"),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .pre_admit(r.person_id, bed.id, Some("hl7v2_a05"), &r.audit)
        .await
    {
        Ok(e) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "pre-admitted person={} encounter={} bed={}",
                r.person_id, e.id, bed.id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/cancel-pre-admit",
    tag = "Hl7",
    summary = "Cancel pre admit",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_pre_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let bed_code = match r.message.segment("PV1").and_then(extract_bed_code_from_pv1) {
        Some(c) => c,
        None => return ae(&r.sender, &r.control_id, "PV1-3.3 (bed) is required"),
    };
    let bed = match BedRepository::find_by_code(&state.db, &bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("bed code {bed_code:?} not found"),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .cancel_pre_admit(r.person_id, bed.id, Some("hl7v2_a38"), &r.audit)
        .await
    {
        Ok(e) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "cancelled pre-admit person={} encounter={} bed={}",
                r.person_id, e.id, bed.id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/change-to-inpatient",
    tag = "Hl7",
    summary = "Change to inpatient",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn change_to_inpatient(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let bed_code = match r.message.segment("PV1").and_then(extract_bed_code_from_pv1) {
        Some(c) => c,
        None => return ae(&r.sender, &r.control_id, "PV1-3.3 (bed) is required"),
    };
    let bed = match BedRepository::find_by_code(&state.db, &bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("bed code {bed_code:?} not found"),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .change_to_inpatient(r.person_id, bed.id, Some("hl7v2_a06"), &r.audit)
        .await
    {
        Ok(result) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "promoted person={} encounter={} bed={}",
                r.person_id, result.encounter.id, bed.id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/change-to-outpatient",
    tag = "Hl7",
    summary = "Change to outpatient",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn change_to_outpatient(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, r.person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("no open admission for person {}", r.person_id),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .change_to_outpatient(admission.id, Some("hl7v2_a07"), &r.audit)
        .await
    {
        Ok(e) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "demoted person={} encounter={}",
                r.person_id, e.id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/update",
    tag = "Hl7",
    summary = "Update",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    // A08 overwrites the upstream person from the new PID. Use
    // `PersonClient::update` against the resolved person id.
    let Some(client) = need_person_client(&state) else {
        return ae(
            &r.sender,
            &r.control_id,
            "PERSON_SERVICE_URL unset; PAS cannot push update",
        );
    };
    let create = pid_to_person_create(&r.pid);
    match client.update(r.person_id, create, &r.upstream).await {
        Ok(_) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!("updated person={}", r.person_id)),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/cancel-admit",
    tag = "Hl7",
    summary = "Cancel admit",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, r.person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("no open admission for person {}", r.person_id),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .cancel_admission(admission.id, Some("hl7v2_a11"), &r.audit)
        .await
    {
        Ok(_) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "cancelled admission={} person={}",
                admission.id, r.person_id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/cancel-transfer",
    tag = "Hl7",
    summary = "Cancel transfer",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, r.person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("no open admission for person {}", r.person_id),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .cancel_transfer(admission.id, Some("hl7v2_a12"), &r.audit)
        .await
    {
        Ok(_) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!("reverted transfer on admission={}", admission.id)),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/cancel-discharge",
    tag = "Hl7",
    summary = "Cancel discharge",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn cancel_discharge(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let (admission, _discharge) =
        match AdmissionRepository::find_most_recently_discharged_for_patient(&state.db, r.person_id)
            .await
        {
            Ok(Some(pair)) => pair,
            Ok(None) => {
                return ae(
                    &r.sender,
                    &r.control_id,
                    &format!("no discharged admission found for person {}", r.person_id),
                );
            }
            Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
        };
    let admission_id = admission.id;
    match state.adt.cancel_discharge(admission_id, &r.audit).await {
        Ok(_) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "reinstated discharge on admission={admission_id} person={}",
                r.person_id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/leave-start",
    tag = "Hl7",
    summary = "Leave start",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn leave_start(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, r.person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("no open admission for person {}", r.person_id),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .start_leave(admission.id, Some("hl7v2_a21"), &r.audit)
        .await
    {
        Ok(e) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "leave-start admission={} encounter={}",
                admission.id, e.id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/leave-end",
    tag = "Hl7",
    summary = "Leave end",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn leave_end(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, r.person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("no open admission for person {}", r.person_id),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    match state
        .adt
        .end_leave(admission.id, Some("hl7v2_a22"), &r.audit)
        .await
    {
        Ok(e) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "leave-end admission={} encounter={}",
                admission.id, e.id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/delete-patient",
    tag = "Hl7",
    summary = "Delete patient",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn delete_patient(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let Some(client) = need_person_client(&state) else {
        return ae(
            &r.sender,
            &r.control_id,
            "PERSON_SERVICE_URL unset; cannot delete upstream person",
        );
    };
    match client.delete(r.person_id, &r.upstream).await {
        Ok(()) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!("deleted person={}", r.person_id)),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/merge",
    tag = "Hl7",
    summary = "Merge",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn merge(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    // A40 carries the SURVIVOR in PID and the SOURCE MRN in MRG-1. The
    // upstream merge endpoint takes two ids — we resolve PID via
    // dedup-or-create and MRG via a search on the source MRN.
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let mrg = match r.message.segment("MRG") {
        Some(s) => s,
        None => return ae(&r.sender, &r.control_id, "MRG segment missing"),
    };
    let source_mrn = match extract_mrn_from_mrg(mrg) {
        Some(s) => s,
        None => return ae(&r.sender, &r.control_id, "MRG-1.1 (source MRN) is required"),
    };
    let Some(client) = need_person_client(&state) else {
        return ae(
            &r.sender,
            &r.control_id,
            "PERSON_SERVICE_URL unset; cannot resolve MRG source",
        );
    };
    // Look up the source by MRN. We do NOT dedup-or-create the source —
    // it must already exist or the merge is rejected (no person to merge
    // from).
    let hits = match client.search(&source_mrn, 20, &r.upstream).await {
        Ok(h) => h,
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    let source_id = match hits
        .iter()
        .find(|p| {
            p.data
                .get("identifiers")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter().any(|id| {
                        id.get("value")
                            .and_then(|v| v.as_str())
                            .map(|s| s == source_mrn)
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        })
        .map(|p| p.id)
    {
        Some(id) => id,
        None => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("source MRN {source_mrn:?} not found upstream"),
            );
        }
    };
    if source_id == r.person_id {
        return ae(
            &r.sender,
            &r.control_id,
            "MRG-1 resolved to the same person as PID (self-merge)",
        );
    }
    match client.merge(source_id, r.person_id, &r.upstream).await {
        Ok(_) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "merged source={source_id} into target={}",
                r.person_id
            )),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}
