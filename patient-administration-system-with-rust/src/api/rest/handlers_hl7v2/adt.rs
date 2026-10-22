//! Part of [`super`]: adt.

use super::*;

#[utoipa::path(
    post,
    path = "/api/hl7/v2/patient",
    tag = "Hl7",
    summary = "Add person",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn add_person(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let h = msh_header(&msg);
    let pid = match msg.segment("PID") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "PID segment missing"),
    };
    let Some(client) = need_person_client(&state) else {
        return ae(
            &h.sender,
            &h.control_id,
            "PERSON_SERVICE_URL unset; PAS cannot resolve PID",
        );
    };
    let ctx = upstream_ctx(&request_audit_ctx(&headers));
    match dedup_or_create_person(client, pid, &ctx).await {
        Ok((id, created)) => {
            let diag = if created {
                format!("created person {id}")
            } else {
                format!("matched existing person {id}")
            };
            aa(&h.sender, &h.control_id, Some(&diag))
        }
        Err(e) => ae(&h.sender, &h.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/admit",
    tag = "Hl7",
    summary = "Admit",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn admit(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let h = msh_header(&msg);
    let pid = match msg.segment("PID") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "PID segment missing"),
    };
    let pv1 = match msg.segment("PV1") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "PV1 segment missing"),
    };
    let bed_code = match extract_bed_code_from_pv1(pv1) {
        Some(c) => c,
        None => return ae(&h.sender, &h.control_id, "PV1-3.3 (bed code) is required"),
    };
    let Some(client) = need_person_client(&state) else {
        return ae(
            &h.sender,
            &h.control_id,
            "PERSON_SERVICE_URL unset; PAS cannot resolve PID",
        );
    };
    let audit = request_audit_ctx(&headers);
    let ctx = upstream_ctx(&audit);
    let (person_id, _) = match dedup_or_create_person(client, pid, &ctx).await {
        Ok(r) => r,
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    let bed = match BedRepository::find_by_code(&state.db, &bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return ae(
                &h.sender,
                &h.control_id,
                &format!("bed code {bed_code:?} not found"),
            );
        }
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    match state.adt.admit(person_id, bed.id, &audit).await {
        Ok(result) => aa(
            &h.sender,
            &h.control_id,
            Some(&format!(
                "admitted person={person_id} bed={} admission={}",
                bed.id, result.admission.id
            )),
        ),
        Err(e) => ae(&h.sender, &h.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/transfer",
    tag = "Hl7",
    summary = "Transfer",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let h = msh_header(&msg);
    let pid = match msg.segment("PID") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "PID segment missing"),
    };
    let pv1 = match msg.segment("PV1") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "PV1 segment missing"),
    };
    let bed_code = match extract_bed_code_from_pv1(pv1) {
        Some(c) => c,
        None => {
            return ae(
                &h.sender,
                &h.control_id,
                "PV1-3.3 (destination bed) is required",
            );
        }
    };
    let Some(client) = need_person_client(&state) else {
        return ae(
            &h.sender,
            &h.control_id,
            "PERSON_SERVICE_URL unset; PAS cannot resolve PID",
        );
    };
    let audit = request_audit_ctx(&headers);
    let ctx = upstream_ctx(&audit);
    let (person_id, _) = match dedup_or_create_person(client, pid, &ctx).await {
        Ok(r) => r,
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &h.sender,
                &h.control_id,
                &format!("no open admission for person {person_id}"),
            );
        }
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    let bed = match BedRepository::find_by_code(&state.db, &bed_code).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            return ae(
                &h.sender,
                &h.control_id,
                &format!("bed code {bed_code:?} not found"),
            );
        }
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    match state.adt.transfer(admission.id, bed.id, &audit).await {
        Ok(_) => aa(
            &h.sender,
            &h.control_id,
            Some(&format!(
                "transferred person={person_id} admission={} to bed={}",
                admission.id, bed.id
            )),
        ),
        Err(e) => ae(&h.sender, &h.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/discharge",
    tag = "Hl7",
    summary = "Discharge",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn discharge(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let h = msh_header(&msg);
    let pid = match msg.segment("PID") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "PID segment missing"),
    };
    let Some(client) = need_person_client(&state) else {
        return ae(
            &h.sender,
            &h.control_id,
            "PERSON_SERVICE_URL unset; PAS cannot resolve PID",
        );
    };
    let audit = request_audit_ctx(&headers);
    let ctx = upstream_ctx(&audit);
    let (person_id, _) = match dedup_or_create_person(client, pid, &ctx).await {
        Ok(r) => r,
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    let admission = match AdmissionRepository::find_open_for_patient(&state.db, person_id).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &h.sender,
                &h.control_id,
                &format!("no open admission for person {person_id}"),
            );
        }
        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
    };
    match state.adt.discharge(admission.id, &audit).await {
        Ok(_) => aa(
            &h.sender,
            &h.control_id,
            Some(&format!(
                "discharged person={person_id} admission={}",
                admission.id
            )),
        ),
        Err(e) => ae(&h.sender, &h.control_id, &client_text(&e)),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/register",
    tag = "Hl7",
    summary = "Register",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    // PV1-2 chooses Emergency vs Outpatient. Missing PV1 → Outpatient.
    let class_str = match r.message.segment("PV1") {
        Some(pv1) => pv1_class_or_outpatient(pv1),
        None => "Outpatient",
    };
    let class = if class_str == "Emergency" {
        EncounterClass::Emergency
    } else {
        EncounterClass::Outpatient
    };
    // Build an Encounter directly in `Arrived` status (no bed allocated).
    let mut enc = Encounter::new(r.person_id, class);
    enc.status = EncounterStatus::Arrived;
    let enc_clone = enc.clone();
    let audit_clone = r.audit.clone();
    let res = state
        .db
        .transaction::<_, Encounter, crate::Error>(|txn| {
            Box::pin(async move {
                let e = EncounterRepository::create(txn, &enc_clone).await?;
                AuditLogRepository::log(
                    txn,
                    "encounter",
                    e.id,
                    "register_via_hl7v2_a04",
                    None,
                    Some(serde_json::to_value(&e).unwrap_or_default()),
                    &audit_clone,
                )
                .await?;
                OutboxRepository::publish(
                    txn,
                    "EncounterRegistered",
                    &serde_json::json!({
                        "encounter_id": e.id,
                        "person_id": e.person_id,
                        "class": format!("{:?}", e.class),
                        "source": "hl7v2_a04",
                    }),
                )
                .await?;
                Ok(e)
            })
        })
        .await;
    match res {
        Ok(e) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!(
                "registered encounter={} person={} class={}",
                e.id, r.person_id, class_str
            )),
        ),
        Err(e) => ae(
            &r.sender,
            &r.control_id,
            &crate::Error::from(e).client_message(),
        ),
    }
}
