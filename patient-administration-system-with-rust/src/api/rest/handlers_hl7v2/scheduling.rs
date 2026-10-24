//! Part of [`super`]: scheduling.

use super::*;
use sea_orm::TransactionTrait;

#[utoipa::path(
    post,
    path = "/api/hl7/v2/schedule-book",
    tag = "Hl7",
    summary = "Schedule book",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn schedule_book(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let sch = match r.message.segment("SCH") {
        Some(s) => parse_sch(s),
        None => return ae(&r.sender, &r.control_id, "SCH segment missing"),
    };
    let Some(start) = sch.start else {
        return ae(
            &r.sender,
            &r.control_id,
            "SCH-11 (start datetime) is required",
        );
    };
    let duration = sch.duration_minutes.unwrap_or(30);
    let end = start + chrono::Duration::minutes(i64::from(duration));

    let person_id = r.person_id;
    let audit_clone = r.audit.clone();
    let result = state
        .db
        .transaction::<_, Appointment, crate::Error>(|txn| {
            Box::pin(async move {
                use crate::db::repositories::appointment::AppointmentRepository;
                let overlapping =
                    AppointmentRepository::find_overlapping_for_patient(txn, person_id, start, end)
                        .await?;
                if !overlapping.is_empty() {
                    return Err(crate::Error::conflict(format!(
                        "appointment overlap for person {person_id} at {start}"
                    )));
                }
                let mut appt = Appointment::new(person_id, start, end);
                appt.status = AppointmentStatus::Booked;
                appt.reason = sch.reason.clone();
                let saved = AppointmentRepository::create(txn, &appt).await?;
                AuditLogRepository::log(
                    txn,
                    "appointment",
                    saved.id,
                    "book_via_hl7v2_s12",
                    None,
                    Some(serde_json::to_value(&saved).unwrap_or_default()),
                    &audit_clone,
                )
                .await?;
                OutboxRepository::publish(
                    txn,
                    "AppointmentBooked",
                    &serde_json::json!({
                        "appointment_id": saved.id,
                        "person_id": person_id,
                        "source": "hl7v2_s12",
                    }),
                )
                .await?;
                Ok(saved)
            })
        })
        .await;
    match result {
        Ok(a) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!("filler={} person={person_id}", a.id)),
        ),
        Err(e) => ae(
            &r.sender,
            &r.control_id,
            &crate::Error::from(e).client_message(),
        ),
    }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/schedule-reschedule",
    tag = "Hl7",
    summary = "Schedule reschedule",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn schedule_reschedule(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let sch = match r.message.segment("SCH") {
        Some(s) => parse_sch(s),
        None => return ae(&r.sender, &r.control_id, "SCH segment missing"),
    };
    let Some(filler) = sch.filler_id else {
        return ae(
            &r.sender,
            &r.control_id,
            "SCH-2 (filler appointment id) is required",
        );
    };
    let Some(start) = sch.start else {
        return ae(
            &r.sender,
            &r.control_id,
            "SCH-11 (start datetime) is required",
        );
    };
    let duration = sch.duration_minutes.unwrap_or(30);
    let end = start + chrono::Duration::minutes(i64::from(duration));
    use crate::db::repositories::appointment::AppointmentRepository;
    let existing = match AppointmentRepository::find_by_id(&state.db, filler).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("filler {filler} not found"),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    if matches!(
        existing.status,
        AppointmentStatus::Cancelled | AppointmentStatus::NoShow | AppointmentStatus::Fulfilled
    ) {
        return ae(
            &r.sender,
            &r.control_id,
            &format!("appointment {filler} is terminal ({:?})", existing.status),
        );
    }
    let conflicts = match AppointmentRepository::find_overlapping_for_patient_excluding(
        &state.db,
        existing.person_id,
        start,
        end,
        filler,
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    if !conflicts.is_empty() {
        return ae(
            &r.sender,
            &r.control_id,
            &format!("overlap with appointment {}", conflicts[0].id),
        );
    }
    let mut updated = existing.clone();
    updated.start_datetime = start;
    updated.end_datetime = end;
    updated.updated_at = chrono::Utc::now();
    // The change, its audit row and its event commit together, or none of them is kept (NFR-25).
    if let Err(e) = async {
        let txn = state.db.begin().await.map_err(crate::Error::Database)?;
        let saved = AppointmentRepository::update(&txn, &updated).await?;
        AuditLogRepository::log(
            &txn,
            "appointment",
            saved.id,
            "reschedule_via_hl7v2_s13",
            Some(serde_json::to_value(&existing).unwrap_or_default()),
            Some(serde_json::to_value(&saved).unwrap_or_default()),
            &r.audit,
        )
        .await?;
        OutboxRepository::publish(
            &txn,
            "AppointmentRescheduled",
            &serde_json::json!({ "appointment_id": saved.id, "source": "hl7v2_s13" }),
        )
        .await?;
        txn.commit().await.map_err(crate::Error::Database)?;
        Ok::<_, crate::Error>(())
    }
    .await
    {
        return ae(&r.sender, &r.control_id, &client_text(&e));
    }
    aa(
        &r.sender,
        &r.control_id,
        Some(&format!("rescheduled filler={filler}")),
    )
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/schedule-modify",
    tag = "Hl7",
    summary = "Schedule modify",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn schedule_modify(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let sch = match r.message.segment("SCH") {
        Some(s) => parse_sch(s),
        None => return ae(&r.sender, &r.control_id, "SCH segment missing"),
    };
    let Some(filler) = sch.filler_id else {
        return ae(
            &r.sender,
            &r.control_id,
            "SCH-2 (filler appointment id) is required",
        );
    };
    use crate::db::repositories::appointment::AppointmentRepository;
    let existing = match AppointmentRepository::find_by_id(&state.db, filler).await {
        Ok(Some(a)) => a,
        Ok(None) => {
            return ae(
                &r.sender,
                &r.control_id,
                &format!("filler {filler} not found"),
            );
        }
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };
    if matches!(
        existing.status,
        AppointmentStatus::Cancelled | AppointmentStatus::NoShow | AppointmentStatus::Fulfilled
    ) {
        return ae(
            &r.sender,
            &r.control_id,
            &format!("appointment {filler} is terminal ({:?})", existing.status),
        );
    }
    let mut updated = existing.clone();
    updated.reason = sch.reason.clone();
    updated.updated_at = chrono::Utc::now();
    // The change, its audit row and its event commit together, or none of them is kept (NFR-25).
    if let Err(e) = async {
        let txn = state.db.begin().await.map_err(crate::Error::Database)?;
        let saved = AppointmentRepository::update(&txn, &updated).await?;
        AuditLogRepository::log(
            &txn,
            "appointment",
            saved.id,
            "modify_via_hl7v2_s14",
            Some(serde_json::to_value(&existing).unwrap_or_default()),
            Some(serde_json::to_value(&saved).unwrap_or_default()),
            &r.audit,
        )
        .await?;
        OutboxRepository::publish(
            &txn,
            "AppointmentModified",
            &serde_json::json!({ "appointment_id": saved.id, "source": "hl7v2_s14" }),
        )
        .await?;
        txn.commit().await.map_err(crate::Error::Database)?;
        Ok::<_, crate::Error>(())
    }
    .await
    {
        return ae(&r.sender, &r.control_id, &client_text(&e));
    }
    aa(
        &r.sender,
        &r.control_id,
        Some(&format!("modified filler={filler}")),
    )
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/schedule-cancel",
    tag = "Hl7",
    summary = "Schedule cancel",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn schedule_cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let sch = match r.message.segment("SCH") {
        Some(s) => parse_sch(s),
        None => return ae(&r.sender, &r.control_id, "SCH segment missing"),
    };
    let Some(filler) = sch.filler_id else {
        return ae(
            &r.sender,
            &r.control_id,
            "SCH-2 (filler appointment id) is required",
        );
    };
    match state
        .scheduling
        .cancel(filler, CancellationReason::PatientRequest, &r.audit)
        .await
    {
        Ok(_) => aa(
            &r.sender,
            &r.control_id,
            Some(&format!("cancelled filler={filler}")),
        ),
        Err(e) => ae(&r.sender, &r.control_id, &client_text(&e)),
    }
}
