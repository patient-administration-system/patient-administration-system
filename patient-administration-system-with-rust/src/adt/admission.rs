//! Part of [`super`]: admission.

use super::*;

impl AdtService {
    /// Admit a patient to a bed.
    ///
    /// Transactional steps:
    ///
    /// 1. `SELECT … FOR UPDATE` the target bed.
    /// 2. Assert `Bed.status == Available`.
    /// 3. Create the `Encounter` (Inpatient, status transitions
    ///    Planned → Arrived → InProgress).
    /// 4. Flip the bed to `Occupied`.
    /// 5. Insert the `BedAssignment` and `Admission` rows.
    /// 6. Write audit and outbox rows.
    /// 7. Commit. After commit, publish a best-effort `EncounterAdmitted`
    ///    event via the configured publisher.
    pub async fn admit(
        &self,
        person_id: Uuid,
        bed_id: Uuid,
        ctx: &UserContext,
    ) -> Result<AdmissionResult> {
        let ctx_clone = ctx.clone();
        let res = self
            .db
            .transaction::<_, AdmissionResult, Error>(|txn| {
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

                    // Create encounter and advance status to InProgress.
                    let mut enc = Encounter::new(person_id, EncounterClass::Inpatient);
                    enc.status = enc.status.try_transition_to(EncounterStatus::Arrived)?;
                    enc.status = enc.status.try_transition_to(EncounterStatus::InProgress)?;
                    let enc = EncounterRepository::create(txn, &enc).await?;

                    // Mark bed Occupied.
                    BedRepository::update_status(txn, bed_id, BedStatus::Occupied).await?;

                    // Insert the active bed assignment.
                    let ba = BedAssignment::new(enc.id, bed_id);
                    let ba = AdmissionRepository::create_bed_assignment(txn, &ba).await?;

                    // Insert the admission record.
                    let adm = Admission::new(enc.id, bed_id);
                    let adm = AdmissionRepository::create(txn, &adm).await?;

                    // Audit + outbox in the same transaction.
                    AuditLogRepository::log(
                        txn,
                        "encounter",
                        enc.id,
                        "admit",
                        None,
                        Some(serde_json::to_value(&enc).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "EncounterAdmitted",
                        &serde_json::json!({
                            "encounter_id": enc.id,
                            "person_id": person_id,
                            "bed_id": bed_id,
                            "admission_id": adm.id,
                        }),
                    )
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

        // Best-effort publish after commit. The outbox row is the durable
        // record; this is a convenience for in-process consumers.
        self.publisher
            .publish(DomainEvent::new(
                "EncounterAdmitted",
                serde_json::json!({
                    "encounter_id": res.encounter.id,
                    "admission_id": res.admission.id,
                }),
            ))
            .await
            .log_warn("in-process event");
        Ok(res)
    }

    /// Pre-admit a patient (HL7 v2 ADT^A05 semantics): reserve a bed and
    /// create a `Planned`-status inpatient encounter without actually
    /// admitting the patient yet.
    ///
    /// Transactional steps: lock the destination bed (must be
    /// `Available`), flip it to `Reserved` via the regular state-machine
    /// transition (`Available → Reserved` is legal), create an
    /// `Encounter` in `Planned` status, write audit + outbox, commit.
    ///
    /// Unlike `admit`, this does *not* create an `Admission` row or a
    /// `BedAssignment` — the patient isn't physically in the bed yet.
    /// The bed reservation is tracked only via the bed's `Reserved`
    /// status. A subsequent ADT^A01 admit for the same patient is
    /// expected to land on a different code path that's aware of the
    /// pre-existing planned encounter (future work).
    pub async fn pre_admit(
        &self,
        person_id: Uuid,
        bed_id: Uuid,
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
                    let bed = BedRepository::select_for_update(txn, bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {bed_id}")))?;
                    if bed.status != BedStatus::Available {
                        return Err(Error::conflict(format!(
                            "bed {bed_id} is not available for pre-admit (status={:?})",
                            bed.status
                        )));
                    }
                    BedRepository::update_status(txn, bed_id, BedStatus::Reserved).await?;
                    let enc = Encounter::new(person_id, EncounterClass::Inpatient);
                    let enc = EncounterRepository::create(txn, &enc).await?;
                    AuditLogRepository::log(
                        txn,
                        "encounter",
                        enc.id,
                        "pre_admit",
                        None,
                        Some(serde_json::to_value(&enc).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "encounter_id": enc.id,
                        "person_id": person_id,
                        "bed_id": bed_id,
                    });
                    if let Some(s) = source_for_txn.as_deref() {
                        payload["source"] = serde_json::Value::String(s.to_string());
                    }
                    OutboxRepository::publish(txn, "EncounterPreAdmitted", &payload).await?;
                    Ok(enc)
                })
            })
            .await
            .map_err(Error::from)?;

        self.publisher
            .publish(DomainEvent::new(
                "EncounterPreAdmitted",
                serde_json::json!({
                    "encounter_id": res.id,
                    "person_id": person_id,
                    "bed_id": bed_id,
                }),
            ))
            .await
            .log_warn("in-process event");
        Ok(res)
    }

    /// Cancel a pre-admission (HL7 v2 ADT^A38 semantics): release the
    /// bed reservation and cancel the Planned inpatient encounter.
    ///
    /// Transactional steps: lock the bed (must be `Reserved` — anything
    /// else means the reservation already advanced or never existed),
    /// locate the patient's most-recent Planned inpatient encounter
    /// (404 if none), flip bed → `Available` via the regular state-
    /// machine transition (`Reserved → Available` is legal), set the
    /// encounter status → `Cancelled` via the regular state machine
    /// (`Planned → Cancelled` is legal), write audit + outbox, commit.
    pub async fn cancel_pre_admit(
        &self,
        person_id: Uuid,
        bed_id: Uuid,
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
                    let bed = BedRepository::select_for_update(txn, bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {bed_id}")))?;
                    if bed.status != BedStatus::Reserved {
                        return Err(Error::conflict(format!(
                            "bed {bed_id} is in status {:?}; expected Reserved for cancel-pre-admit",
                            bed.status
                        )));
                    }
                    let enc =
                        EncounterRepository::find_latest_planned_inpatient_for_patient(
                            txn,
                            person_id,
                        )
                        .await?
                        .ok_or_else(|| {
                            Error::not_found(format!(
                                "patient {person_id} has no planned inpatient encounter to cancel"
                            ))
                        })?;
                    BedRepository::update_status(txn, bed_id, BedStatus::Available).await?;
                    let enc = EncounterRepository::set_status(
                        txn,
                        enc.id,
                        EncounterStatus::Cancelled,
                    )
                    .await?;
                    AuditLogRepository::log(
                        txn,
                        "encounter",
                        enc.id,
                        "cancel_pre_admit",
                        None,
                        Some(serde_json::json!({
                            "person_id": person_id,
                            "bed_id": bed_id,
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "encounter_id": enc.id,
                        "person_id": person_id,
                        "bed_id": bed_id,
                    });
                    if let Some(s) = source_for_txn.as_deref() {
                        payload["source"] = serde_json::Value::String(s.to_string());
                    }
                    OutboxRepository::publish(txn, "EncounterPreAdmitCancelled", &payload).await?;
                    Ok(enc)
                })
            })
            .await
            .map_err(Error::from)?;

        let mut in_mem = serde_json::json!({
            "encounter_id": res.id,
            "person_id": person_id,
            "bed_id": bed_id,
        });
        if let Some(s) = source_owned.as_deref() {
            in_mem["source"] = serde_json::Value::String(s.to_string());
        }
        self.publisher
            .publish(DomainEvent::new("EncounterPreAdmitCancelled", in_mem))
            .await
            .log_warn("in-process event");
        Ok(res)
    }

    /// Cancel a wrong admission (HL7 v2 ADT^A11 semantics).
    ///
    /// Transactional steps: load the admission, release the active
    /// `BedAssignment` (if any), flip the bed to `Cleaning` (the regular
    /// post-Occupied transition — operators can subsequently mark the bed
    /// `Available` if no cleaning is actually required), force the encounter
    /// to `Cancelled` (allowed from `InProgress` by the regular state
    /// machine), write audit + outbox, commit. The `admissions` row is
    /// preserved so the cancelled admission is still visible in the
    /// patient's history.
    pub async fn cancel_admission(
        &self,
        admission_id: Uuid,
        reason: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Admission> {
        let ctx_clone = ctx.clone();
        let reason_owned = reason.map(std::string::ToString::to_string);
        let res = self
            .db
            .transaction::<_, Admission, Error>(|txn| {
                let reason_for_txn = reason_owned.clone();
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;

                    // Release the active bed assignment (if any) and free
                    // the bed by walking it through Cleaning. We respect
                    // the BedStatus state machine because Occupied → Cleaning
                    // is a legal transition; an operator can mark the bed
                    // Available immediately afterwards if no cleaning is
                    // actually required.
                    let freed_bed_id = if let Some(ba) =
                        AdmissionRepository::find_active_by_encounter(txn, adm.encounter_id).await?
                    {
                        AdmissionRepository::release_bed_assignment(txn, ba.id).await?;
                        BedRepository::update_status(txn, ba.bed_id, BedStatus::Cleaning).await?;
                        Some(ba.bed_id)
                    } else {
                        None
                    };

                    // Move the encounter to Cancelled. This goes through the
                    // regular state machine — Planned/Arrived/InProgress/
                    // OnLeave all permit a transition to Cancelled.
                    EncounterRepository::set_status(
                        txn,
                        adm.encounter_id,
                        EncounterStatus::Cancelled,
                    )
                    .await?;

                    AuditLogRepository::log(
                        txn,
                        "admission",
                        admission_id,
                        "cancel_admit",
                        None,
                        Some(serde_json::to_value(&adm).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "admission_id": admission_id,
                        "encounter_id": adm.encounter_id,
                        "bed_id": freed_bed_id,
                    });
                    if let Some(r) = reason_for_txn.as_deref() {
                        payload["reason"] = serde_json::Value::String(r.to_string());
                    }
                    OutboxRepository::publish(txn, "EncounterCancelled", &payload).await?;

                    Ok(adm)
                })
            })
            .await
            .map_err(Error::from)?;

        let mut in_mem = serde_json::json!({
            "admission_id": admission_id,
            "encounter_id": res.encounter_id,
        });
        if let Some(r) = reason_owned.as_deref() {
            in_mem["reason"] = serde_json::Value::String(r.to_string());
        }
        self.publisher
            .publish(DomainEvent::new("EncounterCancelled", in_mem))
            .await
            .log_warn("in-process event");
        Ok(res)
    }
}
