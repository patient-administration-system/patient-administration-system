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
    if let Err(ack) = check_master_file(&msg, &h, "PRA") {
        return ack;
    }
    let Some(client) = need_worker_client(&state) else {
        return ae(
            &h.sender,
            &h.control_id,
            "WORKER_SERVICE_URL unset; cannot apply staff master file",
        );
    };
    let items = match parse_staff_items(&msg, &h) {
        Ok(items) => items,
        Err(ack) => return ack,
    };
    let up = upstream_ctx(&request_audit_ctx(&headers));
    let mut applied = 0usize;
    for item in &items {
        if let Err(why) = apply_staff_item(client, item, &up).await {
            return ae(&h.sender, &h.control_id, &why);
        }
        applied += 1;
    }
    aa(
        &h.sender,
        &h.control_id,
        Some(&format!("staff_records_applied={applied}")),
    )
}

/// The master file an MFN message is about (`MFI-1`) must be the one this endpoint takes.
fn check_master_file(
    msg: &crate::hl7v2::Message,
    h: &MshHeader,
    expected: &str,
) -> std::result::Result<(), Hl7Response> {
    let Some(mfi) = msg.segment("MFI") else {
        return Err(ae(&h.sender, &h.control_id, "MFI segment missing"));
    };
    let file_id = mfi.raw(1).trim().to_ascii_uppercase();
    if file_id != expected {
        return Err(ae(
            &h.sender,
            &h.control_id,
            &format!("MFI-1 file id {file_id:?} not supported ({expected} only)"),
        ));
    }
    Ok(())
}

/// The `MFE`+`STF` pairs of an MFN^M02 message, or the ACK to send back when the message is not one.
fn parse_staff_items(
    msg: &crate::hl7v2::Message,
    h: &MshHeader,
) -> std::result::Result<Vec<crate::hl7v2::pid_translation::StaffItem>, Hl7Response> {
    let mut items: Vec<crate::hl7v2::pid_translation::StaffItem> = Vec::new();
    let mut current_mfe: Option<&crate::hl7v2::Segment> = None;
    for seg in &msg.segments {
        match seg.name.as_str() {
            "MFE" => current_mfe = Some(seg),
            "STF" => {
                let Some(mfe) = current_mfe.take() else {
                    return Err(ae(&h.sender, &h.control_id, "STF without preceding MFE"));
                };
                match parse_mfe_stf(mfe, seg) {
                    Ok(it) => items.push(it),
                    Err(e) => return Err(ae(&h.sender, &h.control_id, &client_text(&e))),
                }
            }
            _ => {}
        }
    }
    if items.is_empty() {
        return Err(ae(&h.sender, &h.control_id, "no MFE+STF pairs in message"));
    }
    Ok(items)
}

/// Apply one staff record (`MAD` add, `MUP` update, `MDL` delete) through the worker service. The error is
/// the text for the ACK.
async fn apply_staff_item(
    client: &dyn WorkerClient,
    item: &crate::hl7v2::pid_translation::StaffItem,
    up: &UpstreamUserContext,
) -> std::result::Result<(), String> {
    let hits = client
        .search(&item.staff_id, 20, up)
        .await
        .map_err(|e| format!("upstream search: {}", client_text(&e)))?;
    let existing = hits
        .into_iter()
        .find(|w| worker_has_identifier(w, &item.staff_id));
    let body = staff_item_to_worker_create(item);
    match item.event_code.as_str() {
        "MAD" => {
            if existing.is_some() {
                return Err(format!("MAD: staff id {:?} already exists", item.staff_id));
            }
            client
                .create(body, up)
                .await
                .map_err(|e| format!("upstream create: {}", client_text(&e)))?;
        }
        "MUP" => {
            let Some(w) = existing else {
                return Err(format!("MUP: staff id {:?} not found", item.staff_id));
            };
            client
                .update(w.id, body, up)
                .await
                .map_err(|e| format!("upstream update: {}", client_text(&e)))?;
        }
        "MDL" => {
            let Some(w) = existing else {
                return Err(format!("MDL: staff id {:?} not found", item.staff_id));
            };
            client
                .delete(w.id, up)
                .await
                .map_err(|e| format!("upstream delete: {}", client_text(&e)))?;
        }
        other => return Err(format!("unsupported MFN event code {other:?}")),
    }
    Ok(())
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
    if let Err(ack) = check_master_file(&msg, &h, "LOC") {
        return ack;
    }
    let items = match parse_location_items(&msg, &h) {
        Ok(items) => items,
        Err(ack) => return ack,
    };
    let audit = request_audit_ctx(&headers);
    let result = state
        .db
        .transaction::<_, usize, crate::Error>(|txn| {
            Box::pin(async move {
                for item in &items {
                    apply_location_item(txn, item, &audit).await?;
                }
                Ok(items.len())
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

/// The `MFE`+`LOC` pairs of an MFN^M05 message, or the ACK to send back when the message is not one.
fn parse_location_items(
    msg: &crate::hl7v2::Message,
    h: &MshHeader,
) -> std::result::Result<Vec<crate::hl7v2::pid_translation::LocationItem>, Hl7Response> {
    let mut items: Vec<crate::hl7v2::pid_translation::LocationItem> = Vec::new();
    let mut current_mfe: Option<&crate::hl7v2::Segment> = None;
    for seg in &msg.segments {
        match seg.name.as_str() {
            "MFE" => current_mfe = Some(seg),
            "LOC" => {
                let Some(mfe) = current_mfe.take() else {
                    return Err(ae(&h.sender, &h.control_id, "LOC without preceding MFE"));
                };
                match parse_mfe_loc(mfe, seg) {
                    Ok(it) => items.push(it),
                    Err(e) => return Err(ae(&h.sender, &h.control_id, &client_text(&e))),
                }
            }
            _ => {}
        }
    }
    if items.is_empty() {
        return Err(ae(&h.sender, &h.control_id, "no MFE+LOC pairs in message"));
    }
    Ok(items)
}

/// Apply one bed record (`MAD` add, `MUP` update, `MDL` take out of service) in the caller's transaction.
async fn apply_location_item(
    txn: &sea_orm::DatabaseTransaction,
    item: &crate::hl7v2::pid_translation::LocationItem,
    audit: &RepoUserContext,
) -> crate::Result<()> {
    use crate::db::repositories::bed::BedRepository;
    use crate::db::repositories::facility::RoomRepository;
    use crate::models::facility::Bed;
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
                audit,
            )
            .await?;
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
                audit,
            )
            .await?;
        }
        "MDL" => {
            let Some(bed) = existing else {
                return Err(crate::Error::not_found(format!(
                    "MDL: bed code {:?} not found",
                    item.bed_code
                )));
            };
            BedRepository::set_status_unchecked(txn, bed.id, BedStatus::OutOfService).await?;
            AuditLogRepository::log(
                txn,
                "bed",
                bed.id,
                "mdl_via_hl7v2_mfn_m05",
                None,
                Some(serde_json::json!({ "bed_code": item.bed_code })),
                audit,
            )
            .await?;
        }
        other => {
            return Err(crate::Error::validation(format!(
                "unsupported MFN event code {other:?}"
            )));
        }
    }
    Ok(())
}
