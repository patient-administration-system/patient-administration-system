//! Part of [`super`]: movements.

use super::*;

impl AdtService {
    /// Transfer an admitted patient to a new bed.
    ///
    /// Transactional steps: end the active `BedAssignment`, free the old bed
    /// (status → `Cleaning`), lock and assert the new bed is `Available`,
    /// mark it `Occupied`, insert the new `BedAssignment` and the `Transfer`
    /// record, then write audit and outbox rows.
    pub async fn transfer(
        &self,
        admission_id: Uuid,
        new_bed_id: Uuid,
        ctx: &UserContext,
    ) -> Result<Transfer> {
        let ctx_clone = ctx.clone();
        let res = self
            .db
            .transaction::<_, Transfer, Error>(|txn| {
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;
                    let active_ba =
                        AdmissionRepository::find_active_by_encounter(txn, adm.encounter_id)
                            .await?
                            .ok_or_else(|| Error::not_found("no active bed assignment"))?;
                    let old_bed_id = active_ba.bed_id;
                    if old_bed_id == new_bed_id {
                        return Err(Error::validation("cannot transfer to the same bed"));
                    }

                    // Lock and verify the new bed is available.
                    let new_bed = BedRepository::select_for_update(txn, new_bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {new_bed_id}")))?;
                    if new_bed.status != BedStatus::Available {
                        return Err(Error::conflict(format!(
                            "bed {new_bed_id} is not available"
                        )));
                    }

                    // Release the old assignment and free the old bed.
                    AdmissionRepository::release_bed_assignment(txn, active_ba.id).await?;
                    BedRepository::update_status(txn, old_bed_id, BedStatus::Cleaning).await?;
                    BedRepository::update_status(txn, new_bed_id, BedStatus::Occupied).await?;

                    // New assignment + transfer record.
                    let new_ba = BedAssignment::new(adm.encounter_id, new_bed_id);
                    let _ = AdmissionRepository::create_bed_assignment(txn, &new_ba).await?;
                    let t = Transfer::new(admission_id, old_bed_id, new_bed_id);
                    let t = AdmissionRepository::create_transfer(txn, &t).await?;

                    AuditLogRepository::log(
                        txn,
                        "admission",
                        admission_id,
                        "transfer",
                        None,
                        Some(serde_json::to_value(&t).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "EncounterTransferred",
                        &serde_json::json!({
                            "admission_id": admission_id,
                            "from_bed_id": old_bed_id,
                            "to_bed_id": new_bed_id,
                        }),
                    )
                    .await?;

                    Ok(t)
                })
            })
            .await
            .map_err(Error::from)?;

        self.publisher
            .publish(DomainEvent::new(
                "EncounterTransferred",
                serde_json::json!({ "admission_id": admission_id }),
            ))
            .await
            .log_warn("in-process event");
        Ok(res)
    }

    /// Promote an ambulatory encounter to an inpatient admission
    /// (HL7 v2 ADT^A06 semantics): find the patient's currently-active
    /// Outpatient or Emergency encounter, allocate a bed, reclassify
    /// the encounter to `Inpatient`, advance the status to
    /// `InProgress` if still `Arrived`, and write a fresh
    /// `Admission` row + `BedAssignment` so subsequent ADT messages
    /// (transfer, discharge) can locate the new inpatient state.
    ///
    /// Transactional steps:
    /// 1. Lock the destination bed (404 if missing; 409 if not
    ///    `Available`).
    /// 2. Find the patient's most-recent active ambulatory encounter
    ///    (404 + AE if none).
    /// 3. Reclassify the encounter to `Inpatient` via
    ///    `EncounterRepository::set_class`.
    /// 4. If the encounter is still `Arrived`, advance to
    ///    `InProgress` via the regular state machine.
    /// 5. Flip the bed to `Occupied`.
    /// 6. Insert `BedAssignment` + `Admission`.
    /// 7. Write audit + outbox `EncounterPromotedToInpatient`.
    pub async fn change_to_inpatient(
        &self,
        person_id: Uuid,
        bed_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
    ) -> Result<AdmissionResult> {
        let ctx_clone = ctx.clone();
        let source_owned = source.map(std::string::ToString::to_string);
        let res = self
            .db
            .transaction::<_, AdmissionResult, Error>(|txn| {
                let source_for_txn = source_owned.clone();
                Box::pin(async move {
                    let bed = BedRepository::select_for_update(txn, bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {bed_id}")))?;
                    if bed.status != BedStatus::Available {
                        return Err(Error::conflict(format!(
                            "bed {bed_id} is not available (status={:?})",
                            bed.status
                        )));
                    }
                    let enc = EncounterRepository::find_latest_active_ambulatory_for_patient(
                        txn, person_id,
                    )
                    .await?
                    .ok_or_else(|| {
                        Error::not_found(format!(
                            "patient {person_id} has no active ambulatory encounter to promote"
                        ))
                    })?;
                    let enc =
                        EncounterRepository::set_class(txn, enc.id, EncounterClass::Inpatient)
                            .await?;
                    let enc = if enc.status == EncounterStatus::Arrived {
                        EncounterRepository::set_status(txn, enc.id, EncounterStatus::InProgress)
                            .await?
                    } else {
                        enc
                    };
                    BedRepository::update_status(txn, bed_id, BedStatus::Occupied).await?;
                    let ba = BedAssignment::new(enc.id, bed_id);
                    let ba = AdmissionRepository::create_bed_assignment(txn, &ba).await?;
                    let adm = Admission::new(enc.id, bed_id);
                    let adm = AdmissionRepository::create(txn, &adm).await?;
                    AuditLogRepository::log(
                        txn,
                        "encounter",
                        enc.id,
                        "change_to_inpatient",
                        None,
                        Some(serde_json::json!({
                            "admission_id": adm.id,
                            "bed_id": bed_id,
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "encounter_id": enc.id,
                        "person_id": person_id,
                        "bed_id": bed_id,
                        "admission_id": adm.id,
                    });
                    if let Some(s) = source_for_txn.as_deref() {
                        payload["source"] = serde_json::Value::String(s.to_string());
                    }
                    OutboxRepository::publish(txn, "EncounterPromotedToInpatient", &payload)
                        .await?;
                    Ok(AdmissionResult {
                        encounter: enc,
                        admission: adm,
                        bed_assignment: ba,
                    })
                })
            })
            .await
            .map_err(Error::from)?;
        let mut in_mem = serde_json::json!({
            "encounter_id": res.encounter.id,
            "admission_id": res.admission.id,
            "person_id": person_id,
            "bed_id": bed_id,
        });
        if let Some(s) = source_owned.as_deref() {
            in_mem["source"] = serde_json::Value::String(s.to_string());
        }
        self.publisher
            .publish(DomainEvent::new("EncounterPromotedToInpatient", in_mem))
            .await
            .log_warn("in-process event");
        Ok(res)
    }

    /// Demote an inpatient encounter to ambulatory (HL7 v2 ADT^A07
    /// semantics): release the bed assignment, reclassify the
    /// encounter from `Inpatient` to `Outpatient`, leave the
    /// encounter status as `InProgress` (the patient is still in
    /// active care — just no longer admitted to a bed). The
    /// `admissions` row is preserved as historical record.
    ///
    /// Transactional steps:
    /// 1. Load the admission (404 if missing).
    /// 2. Release the active `BedAssignment` (404 if none).
    /// 3. Flip bed → `Cleaning` (regular `Occupied → Cleaning`
    ///    transition, same as discharge).
    /// 4. Reclassify the encounter via `EncounterRepository::set_class`
    ///    to `Outpatient`.
    /// 5. Write audit + outbox `EncounterDemotedToOutpatient`.
    pub async fn change_to_outpatient(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Encounter> {
        let ctx_clone = ctx.clone();
        let source_owned = source.map(std::string::ToString::to_string);
        let res = self
            .db
            .transaction::<_, Encounter, Error>(|txn| {
                let source_for_txn = source_owned.clone();
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;
                    let ba = AdmissionRepository::find_active_by_encounter(txn, adm.encounter_id)
                        .await?
                        .ok_or_else(|| {
                            Error::not_found(format!(
                                "admission {admission_id} has no active bed assignment"
                            ))
                        })?;
                    AdmissionRepository::release_bed_assignment(txn, ba.id).await?;
                    BedRepository::update_status(txn, ba.bed_id, BedStatus::Cleaning).await?;
                    let enc = EncounterRepository::set_class(
                        txn,
                        adm.encounter_id,
                        EncounterClass::Outpatient,
                    )
                    .await?;
                    AuditLogRepository::log(
                        txn,
                        "encounter",
                        enc.id,
                        "change_to_outpatient",
                        None,
                        Some(serde_json::json!({
                            "admission_id": admission_id,
                            "released_bed_id": ba.bed_id,
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "encounter_id": enc.id,
                        "person_id": enc.person_id,
                        "admission_id": admission_id,
                        "released_bed_id": ba.bed_id,
                    });
                    if let Some(s) = source_for_txn.as_deref() {
                        payload["source"] = serde_json::Value::String(s.to_string());
                    }
                    OutboxRepository::publish(txn, "EncounterDemotedToOutpatient", &payload)
                        .await?;
                    Ok(enc)
                })
            })
            .await
            .map_err(Error::from)?;
        let mut in_mem = serde_json::json!({
            "encounter_id": res.id,
            "person_id": res.person_id,
            "admission_id": admission_id,
        });
        if let Some(s) = source_owned.as_deref() {
            in_mem["source"] = serde_json::Value::String(s.to_string());
        }
        self.publisher
            .publish(DomainEvent::new("EncounterDemotedToOutpatient", in_mem))
            .await
            .log_warn("in-process event");
        Ok(res)
    }

    /// Cancel a wrong transfer (HL7 v2 ADT^A12 semantics): restore the
    /// patient to the bed they occupied before the most-recent transfer.
    ///
    /// Transactional steps: load the admission, find the most-recent
    /// `Transfer` row for it (404 if none), release the currently-active
    /// `BedAssignment` (must be on the transfer's `to_bed`), lock the
    /// original `from_bed` (must be `Available` or `Cleaning` — anything
    /// else means another patient now owns it and we can't undo the
    /// transfer), flip the destination bed back to `Cleaning` and the
    /// origin bed back to `Occupied` via the `_unchecked` helpers (the
    /// `Cleaning → Occupied` transition is outside the regular state-
    /// machine flow, same exception documented in spec.md §6.4 for A13),
    /// insert a new active `BedAssignment` on the original bed, delete
    /// the cancelled `Transfer` row (the `transfers` table is
    /// operational, not append-only — same as `discharges` for A13),
    /// write audit + outbox, commit.
    pub async fn cancel_transfer(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Admission> {
        let ctx_clone = ctx.clone();
        let source_owned = source.map(std::string::ToString::to_string);
        let res = self
            .db
            .transaction::<_, Admission, Error>(|txn| {
                let source_for_txn = source_owned.clone();
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;

                    let last = AdmissionRepository::find_latest_transfer_for_admission(
                        txn,
                        admission_id,
                    )
                    .await?
                    .ok_or_else(|| {
                        Error::not_found(format!(
                            "admission {admission_id} has no transfer to cancel"
                        ))
                    })?;
                    let from_bed_id = last.from_bed_id;
                    let to_bed_id = last.to_bed_id;

                    // Release the currently-active assignment (which the
                    // application invariants put on `to_bed_id`).
                    let active = AdmissionRepository::find_active_by_encounter(
                        txn,
                        adm.encounter_id,
                    )
                    .await?
                    .ok_or_else(|| Error::not_found("no active bed assignment"))?;
                    if active.bed_id != to_bed_id {
                        return Err(Error::conflict(format!(
                            "active bed_assignment is on {} but latest transfer landed on {to_bed_id}; refusing to cancel-transfer",
                            active.bed_id
                        )));
                    }

                    // Lock the origin bed. Same constraint as A13: it must
                    // still be Available or Cleaning. Anything else means
                    // someone else is now there.
                    let from_bed = BedRepository::select_for_update(txn, from_bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {from_bed_id}")))?;
                    if !matches!(from_bed.status, BedStatus::Available | BedStatus::Cleaning) {
                        return Err(Error::conflict(format!(
                            "origin bed {from_bed_id} is in status {:?}; cannot cancel transfer",
                            from_bed.status
                        )));
                    }

                    AdmissionRepository::release_bed_assignment(txn, active.id).await?;

                    // Flip beds: to_bed → Cleaning (regular Occupied →
                    // Cleaning is legal but the patient just left so this
                    // is the natural post-state); from_bed → Occupied via
                    // the unchecked helper (Cleaning → Occupied is not in
                    // the regular state-machine flow).
                    BedRepository::update_status(txn, to_bed_id, BedStatus::Cleaning).await?;
                    BedRepository::set_status_unchecked(txn, from_bed_id, BedStatus::Occupied)
                        .await?;

                    let ba = BedAssignment::new(adm.encounter_id, from_bed_id);
                    let _ = AdmissionRepository::create_bed_assignment(txn, &ba).await?;

                    let deleted =
                        AdmissionRepository::delete_transfer(txn, last.id).await?;
                    if deleted == 0 {
                        return Err(Error::not_found(format!(
                            "transfer {} disappeared mid-transaction",
                            last.id
                        )));
                    }

                    AuditLogRepository::log(
                        txn,
                        "admission",
                        admission_id,
                        "cancel_transfer",
                        None,
                        Some(serde_json::json!({
                            "transfer_id": last.id,
                            "from_bed_id": from_bed_id,
                            "to_bed_id": to_bed_id,
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "admission_id": admission_id,
                        "encounter_id": adm.encounter_id,
                        "from_bed_id": from_bed_id,
                        "to_bed_id": to_bed_id,
                    });
                    if let Some(s) = source_for_txn.as_deref() {
                        payload["source"] = serde_json::Value::String(s.to_string());
                    }
                    OutboxRepository::publish(txn, "EncounterTransferCancelled", &payload)
                        .await?;

                    Ok(adm)
                })
            })
            .await
            .map_err(Error::from)?;

        let mut in_mem = serde_json::json!({
            "admission_id": admission_id,
            "encounter_id": res.encounter_id,
        });
        if let Some(s) = source_owned.as_deref() {
            in_mem["source"] = serde_json::Value::String(s.to_string());
        }
        self.publisher
            .publish(DomainEvent::new("EncounterTransferCancelled", in_mem))
            .await
            .log_warn("in-process event");
        Ok(res)
    }
}
