//! Part of [`super`]: discharge.

use super::*;

impl AdtService {
    /// Cancel a wrong discharge (HL7 v2 ADT^A13 semantics): reinstate the
    /// admission, put the patient back in the original bed.
    ///
    /// Transactional steps: load the admission + discharge, locate the
    /// most-recently-released `BedAssignment` for the encounter, lock the
    /// original bed (must be in `Available` or `Cleaning` — anything else
    /// means another patient now owns it and we can't undo the discharge),
    /// delete the `discharges` row, force-flip the bed to `Occupied`,
    /// force-set the encounter back to `InProgress`, insert a new active
    /// `BedAssignment`, write audit + outbox, commit.
    ///
    /// **State-machine bypass.** Both the encounter (`Finished` is normally
    /// terminal) and the bed (`Cleaning → Occupied` is not in the regular
    /// flow) are forced via the `_unchecked` repository helpers; this is
    /// the explicit exception documented in spec.md §6.4. Every other
    /// write path keeps using the state-machine-guarded helpers.
    pub async fn cancel_discharge(
        &self,
        admission_id: Uuid,
        ctx: &UserContext,
    ) -> Result<Admission> {
        let ctx_clone = ctx.clone();
        let res = self
            .db
            .transaction::<_, Admission, Error>(|txn| {
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;

                    let prior_ba = AdmissionRepository::find_latest_bed_assignment_for_encounter(
                        txn,
                        adm.encounter_id,
                    )
                    .await?
                    .ok_or_else(|| {
                        Error::not_found(format!(
                            "encounter {} has no bed_assignment history",
                            adm.encounter_id
                        ))
                    })?;
                    let bed_id = prior_ba.bed_id;

                    // Lock the original bed. Allowed reinstatement states
                    // are Available (cleaned + ready) or Cleaning (just
                    // released, no one else has taken it). Occupied means
                    // somebody else is now in it; Reserved / OutOfService
                    // mean the bed is otherwise spoken for.
                    let bed = BedRepository::select_for_update(txn, bed_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("bed {bed_id}")))?;
                    if !matches!(bed.status, BedStatus::Available | BedStatus::Cleaning) {
                        return Err(Error::conflict(format!(
                            "bed {bed_id} is in status {:?}; cannot reinstate admission",
                            bed.status
                        )));
                    }

                    // Physically remove the discharge row. The `discharges`
                    // table is operational (no soft-delete on it per
                    // spec.md §5).
                    let deleted = AdmissionRepository::delete_discharge(txn, admission_id).await?;
                    if deleted == 0 {
                        return Err(Error::not_found(format!(
                            "admission {admission_id} has no discharge to cancel"
                        )));
                    }

                    // Force the bed and encounter back to their pre-discharge
                    // states. Both transitions are outside the regular
                    // state-machine flow.
                    BedRepository::set_status_unchecked(txn, bed_id, BedStatus::Occupied).await?;
                    EncounterRepository::set_status_unchecked(
                        txn,
                        adm.encounter_id,
                        EncounterStatus::InProgress,
                    )
                    .await?;

                    // New active bed_assignment for the original bed. The
                    // partial unique index on bed_assignments(bed_id)
                    // WHERE released_at IS NULL guarantees we can only do
                    // this when no other active assignment exists.
                    let ba = BedAssignment::new(adm.encounter_id, bed_id);
                    let _ = AdmissionRepository::create_bed_assignment(txn, &ba).await?;

                    AuditLogRepository::log(
                        txn,
                        "admission",
                        admission_id,
                        "cancel_discharge",
                        None,
                        Some(serde_json::to_value(&adm).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "EncounterDischargeCancelled",
                        &serde_json::json!({
                            "admission_id": admission_id,
                            "encounter_id": adm.encounter_id,
                            "bed_id": bed_id,
                            "reason": "hl7v2_a13",
                        }),
                    )
                    .await?;

                    Ok(adm)
                })
            })
            .await
            .map_err(Error::from)?;

        self.publisher
            .publish(DomainEvent::new(
                "EncounterDischargeCancelled",
                serde_json::json!({
                    "admission_id": admission_id,
                    "encounter_id": res.encounter_id,
                }),
            ))
            .await
            .log_warn("in-process event");
        Ok(res)
    }

    /// Discharge an admitted patient.
    ///
    /// Transactional steps: finish the encounter, release the active bed
    /// assignment (if any), flip the bed to `Cleaning`, insert the
    /// `Discharge` record, and write audit and outbox rows.
    pub async fn discharge(&self, admission_id: Uuid, ctx: &UserContext) -> Result<Discharge> {
        let ctx_clone = ctx.clone();
        let res = self
            .db
            .transaction::<_, Discharge, Error>(|txn| {
                Box::pin(async move {
                    let adm = AdmissionRepository::find_by_id(txn, admission_id)
                        .await?
                        .ok_or_else(|| Error::not_found(format!("admission {admission_id}")))?;

                    // End the encounter.
                    EncounterRepository::set_status(
                        txn,
                        adm.encounter_id,
                        EncounterStatus::Finished,
                    )
                    .await?;

                    // Release the active bed assignment (if any) and free
                    // the bed.
                    if let Some(ba) =
                        AdmissionRepository::find_active_by_encounter(txn, adm.encounter_id).await?
                    {
                        AdmissionRepository::release_bed_assignment(txn, ba.id).await?;
                        BedRepository::update_status(txn, ba.bed_id, BedStatus::Cleaning).await?;
                    }

                    let d = Discharge::new(admission_id);
                    let d = AdmissionRepository::create_discharge(txn, &d).await?;

                    AuditLogRepository::log(
                        txn,
                        "admission",
                        admission_id,
                        "discharge",
                        None,
                        Some(serde_json::to_value(&d).unwrap_or_default()),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "EncounterDischarged",
                        &serde_json::json!({
                            "admission_id": admission_id,
                            "encounter_id": adm.encounter_id,
                        }),
                    )
                    .await?;

                    Ok(d)
                })
            })
            .await
            .map_err(Error::from)?;

        self.publisher
            .publish(DomainEvent::new(
                "EncounterDischarged",
                serde_json::json!({ "admission_id": admission_id }),
            ))
            .await
            .log_warn("in-process event");
        Ok(res)
    }
}
