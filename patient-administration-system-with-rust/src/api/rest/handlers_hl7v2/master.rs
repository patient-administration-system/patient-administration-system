//! Part of [`super`]: master.

use super::*;

pub(super) fn need_worker_client(state: &AppState) -> Option<&dyn WorkerClient> {
    state.worker_client.as_deref()
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/mfn-staff",
    tag = "Hl7",
    summary = "Mfn staff",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn mfn_staff(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let h = msh_header(&msg);
    let mfi = match msg.segment("MFI") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "MFI segment missing"),
    };
    let file_id = mfi.raw(1).trim().to_ascii_uppercase();
    if file_id != "PRA" {
        return ae(
            &h.sender,
            &h.control_id,
            &format!("MFI-1 file id {file_id:?} not supported (PRA only)"),
        );
    }
    let Some(client) = need_worker_client(&state) else {
        return ae(
            &h.sender,
            &h.control_id,
            "WORKER_SERVICE_URL unset; cannot apply staff master file",
        );
    };
    let audit = request_audit_ctx(&headers);
    let up = upstream_ctx(&audit);
    let mut items: Vec<crate::hl7v2::pid_translation::StaffItem> = Vec::new();
    let mut current_mfe: Option<&crate::hl7v2::Segment> = None;
    for seg in &msg.segments {
        match seg.name.as_str() {
            "MFE" => current_mfe = Some(seg),
            "STF" => {
                if let Some(mfe) = current_mfe.take() {
                    match parse_mfe_stf(mfe, seg) {
                        Ok(it) => items.push(it),
                        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
                    }
                } else {
                    return ae(&h.sender, &h.control_id, "STF without preceding MFE");
                }
            }
            _ => {}
        }
    }
    if items.is_empty() {
        return ae(&h.sender, &h.control_id, "no MFE+STF pairs in message");
    }
    let mut applied = 0usize;
    for item in &items {
        let hits = match client.search(&item.staff_id, 20, &up).await {
            Ok(h) => h,
            Err(e) => {
                return ae(
                    &h.sender,
                    &h.control_id,
                    &format!("upstream search: {}", client_text(&e)),
                );
            }
        };
        let existing = hits
            .into_iter()
            .find(|w| worker_has_identifier(w, &item.staff_id));
        let body = staff_item_to_worker_create(item);
        match item.event_code.as_str() {
            "MAD" => {
                if existing.is_some() {
                    return ae(
                        &h.sender,
                        &h.control_id,
                        &format!("MAD: staff id {:?} already exists", item.staff_id),
                    );
                }
                if let Err(e) = client.create(body, &up).await {
                    return ae(
                        &h.sender,
                        &h.control_id,
                        &format!("upstream create: {}", client_text(&e)),
                    );
                }
                applied += 1;
            }
            "MUP" => {
                let Some(w) = existing else {
                    return ae(
                        &h.sender,
                        &h.control_id,
                        &format!("MUP: staff id {:?} not found", item.staff_id),
                    );
                };
                if let Err(e) = client.update(w.id, body, &up).await {
                    return ae(
                        &h.sender,
                        &h.control_id,
                        &format!("upstream update: {}", client_text(&e)),
                    );
                }
                applied += 1;
            }
            "MDL" => {
                let Some(w) = existing else {
                    return ae(
                        &h.sender,
                        &h.control_id,
                        &format!("MDL: staff id {:?} not found", item.staff_id),
                    );
                };
                if let Err(e) = client.delete(w.id, &up).await {
                    return ae(
                        &h.sender,
                        &h.control_id,
                        &format!("upstream delete: {}", client_text(&e)),
                    );
                }
                applied += 1;
            }
            _ => unreachable!(),
        }
    }
    aa(
        &h.sender,
        &h.control_id,
        Some(&format!("staff_records_applied={applied}")),
    )
}

pub(super) fn worker_has_identifier(w: &crate::upstream::Worker, target: &str) -> bool {
    let Some(arr) = w.data.get("identifiers").and_then(|v| v.as_array()) else {
        return false;
    };
    arr.iter().any(|id| {
        id.get("value")
            .and_then(|v| v.as_str())
            .map(|s| s == target)
            .unwrap_or(false)
    })
}

pub(super) fn staff_item_to_worker_create(
    item: &crate::hl7v2::pid_translation::StaffItem,
) -> WorkerCreate {
    let mut fields = serde_json::Map::new();
    let id_obj = serde_json::json!({
        "use": "official",
        "type": "Other",
        "system": "urn:hl7v2:staff:id",
        "value": item.staff_id,
    });
    fields.insert("identifiers".into(), serde_json::Value::Array(vec![id_obj]));
    let mut name = serde_json::Map::new();
    if !item.family.is_empty() {
        name.insert(
            "family".into(),
            serde_json::Value::String(item.family.clone()),
        );
    }
    let mut given = Vec::new();
    if !item.given.is_empty() {
        given.push(serde_json::Value::String(item.given.clone()));
    }
    if let Some(m) = &item.middle {
        given.push(serde_json::Value::String(m.clone()));
    }
    if !given.is_empty() {
        name.insert("given".into(), serde_json::Value::Array(given));
    }
    if !name.is_empty() {
        fields.insert("name".into(), serde_json::Value::Object(name));
    }
    if let Some(g) = &item.gender {
        fields.insert("gender".into(), serde_json::Value::String(g.clone()));
    }
    if let Some(bd) = &item.birth_date {
        fields.insert("birth_date".into(), serde_json::Value::String(bd.clone()));
    }
    fields.insert("active".into(), serde_json::Value::Bool(item.active));
    WorkerCreate { fields }
}

#[utoipa::path(
    post,
    path = "/api/hl7/v2/mfn-location",
    tag = "Hl7",
    summary = "Mfn location",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn mfn_location(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let msg = match parse_message(&body) {
        Ok(m) => m,
        Err(e) => return ar("unknown", "unknown", &e.to_string()),
    };
    let h = msh_header(&msg);
    let mfi = match msg.segment("MFI") {
        Some(s) => s,
        None => return ae(&h.sender, &h.control_id, "MFI segment missing"),
    };
    let file_id = mfi.raw(1).trim().to_ascii_uppercase();
    if file_id != "LOC" {
        return ae(
            &h.sender,
            &h.control_id,
            &format!("MFI-1 file id {file_id:?} not supported (LOC only)"),
        );
    }
    let audit = request_audit_ctx(&headers);
    let mut items: Vec<crate::hl7v2::pid_translation::LocationItem> = Vec::new();
    let mut current_mfe: Option<&crate::hl7v2::Segment> = None;
    for seg in &msg.segments {
        match seg.name.as_str() {
            "MFE" => current_mfe = Some(seg),
            "LOC" => {
                if let Some(mfe) = current_mfe.take() {
                    match parse_mfe_loc(mfe, seg) {
                        Ok(it) => items.push(it),
                        Err(e) => return ae(&h.sender, &h.control_id, &client_text(&e)),
                    }
                } else {
                    return ae(&h.sender, &h.control_id, "LOC without preceding MFE");
                }
            }
            _ => {}
        }
    }
    if items.is_empty() {
        return ae(&h.sender, &h.control_id, "no MFE+LOC pairs in message");
    }
    use crate::db::repositories::bed::BedRepository;
    use crate::db::repositories::facility::RoomRepository;
    use crate::models::facility::Bed;
    let audit_clone = audit.clone();
    let items_clone = items.clone();
    let result = state
        .db
        .transaction::<_, usize, crate::Error>(|txn| {
            Box::pin(async move {
                let mut applied = 0usize;
                for item in &items_clone {
                    let existing = BedRepository::find_by_code(txn, &item.bed_code).await?;
                    match item.event_code.as_str() {
                        "MAD" => {
                            if existing.is_some() {
                                return Err(crate::Error::conflict(format!(
                                    "MAD: bed code {:?} already exists",
                                    item.bed_code
                                )));
                            }
                            let room_code = item.room_code.as_deref().ok_or_else(|| {
                                crate::Error::validation(
                                    "MFN^M05 MAD: LOC-1.2 (room code) is required to attach a new bed",
                                )
                            })?;
                            let room = RoomRepository::find_by_code(txn, room_code)
                                .await?
                                .ok_or_else(|| {
                                    crate::Error::not_found(format!(
                                        "MFN^M05 MAD: room code {room_code:?} not found"
                                    ))
                                })?;
                            let bed_name = item
                                .bed_name
                                .clone()
                                .unwrap_or_else(|| item.bed_code.clone());
                            let bed = Bed::new(room.id, bed_name, item.bed_code.clone());
                            let created = BedRepository::create(txn, &bed).await?;
                            AuditLogRepository::log(
                                txn,
                                "bed",
                                created.id,
                                "mad_via_hl7v2_mfn_m05",
                                None,
                                Some(serde_json::to_value(&created).unwrap_or_default()),
                                &audit_clone,
                            )
                            .await?;
                            applied += 1;
                        }
                        "MUP" => {
                            let Some(mut bed) = existing else {
                                return Err(crate::Error::not_found(format!(
                                    "MUP: bed code {:?} not found",
                                    item.bed_code
                                )));
                            };
                            if let Some(name) = &item.bed_name {
                                bed.name = name.clone();
                            }
                            bed.updated_at = chrono::Utc::now();
                            let updated = BedRepository::update(txn, &bed).await?;
                            AuditLogRepository::log(
                                txn,
                                "bed",
                                updated.id,
                                "mup_via_hl7v2_mfn_m05",
                                None,
                                Some(serde_json::to_value(&updated).unwrap_or_default()),
                                &audit_clone,
                            )
                            .await?;
                            applied += 1;
                        }
                        "MDL" => {
                            let Some(bed) = existing else {
                                return Err(crate::Error::not_found(format!(
                                    "MDL: bed code {:?} not found",
                                    item.bed_code
                                )));
                            };
                            BedRepository::set_status_unchecked(
                                txn,
                                bed.id,
                                BedStatus::OutOfService,
                            )
                            .await?;
                            AuditLogRepository::log(
                                txn,
                                "bed",
                                bed.id,
                                "mdl_via_hl7v2_mfn_m05",
                                None,
                                Some(serde_json::json!({ "bed_code": item.bed_code })),
                                &audit_clone,
                            )
                            .await?;
                            applied += 1;
                        }
                        _ => unreachable!(),
                    }
                }
                Ok(applied)
            })
        })
        .await;
    match result {
        Ok(n) => aa(
            &h.sender,
            &h.control_id,
            Some(&format!("location_records_applied={n}")),
        ),
        Err(e) => ae(
            &h.sender,
            &h.control_id,
            &crate::Error::from(e).client_message(),
        ),
    }
}
