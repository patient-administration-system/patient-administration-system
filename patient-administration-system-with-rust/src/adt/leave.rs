//! Part of [`super`]: leave.

use super::*;

/// What a leave-of-absence movement does: the encounter status it moves to, and the names of its audit row and
/// outbox event.
pub(super) struct LeaveMove {
    target: EncounterStatus,
    audit_action: &'static str,
    event_type: &'static str,
}

impl AdtService {
    /// Mark a patient as on temporary leave of absence (HL7 v2
    /// ADT^A21 semantics): the encounter moves `InProgress → OnLeave`.
    /// The bed remains `Occupied` — the patient is expected back so
    /// the bed is held for them.
    pub async fn start_leave(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Encounter> {
        self.start_leave_if_unchanged(admission_id, source, None, ctx)
            .await
    }

    /// [`Self::start_leave`] for a caller that read the admission (OFH-1): `expected` is the `updated_at` it saw.
    pub async fn start_leave_if_unchanged(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        expected: Option<chrono::DateTime<chrono::Utc>>,
        ctx: &UserContext,
    ) -> Result<Encounter> {
        self.transition_loa(
            admission_id,
            source,
            expected,
            ctx,
            LeaveMove {
                target: EncounterStatus::OnLeave,
                audit_action: "start_leave",
                event_type: "EncounterLeaveStarted",
            },
        )
        .await
    }

    /// Mark a patient as returned from leave (HL7 v2 ADT^A22
    /// semantics): the encounter moves `OnLeave → InProgress`.
    pub async fn end_leave(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Encounter> {
        self.end_leave_if_unchanged(admission_id, source, None, ctx)
            .await
    }

    /// [`Self::end_leave`] for a caller that read the admission (OFH-1): `expected` is the `updated_at` it saw.
    pub async fn end_leave_if_unchanged(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        expected: Option<chrono::DateTime<chrono::Utc>>,
        ctx: &UserContext,
    ) -> Result<Encounter> {
        self.transition_loa(
            admission_id,
            source,
            expected,
            ctx,
            LeaveMove {
                target: EncounterStatus::InProgress,
                audit_action: "end_leave",
                event_type: "EncounterLeaveEnded",
            },
        )
        .await
    }

    pub(super) async fn transition_loa(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        expected: Option<chrono::DateTime<chrono::Utc>>,
        ctx: &UserContext,
        movement: LeaveMove,
    ) -> Result<Encounter> {
        let LeaveMove {
            target,
            audit_action,
            event_type,
        } = movement;
        let ctx_clone = ctx.clone();
        let source_owned = source.map(std::string::ToString::to_string);
        let res = self
            .db
            .transaction::<_, Encounter, Error>(|txn| {
                let source_for_txn = source_owned.clone();
                Box::pin(async move {
                    let adm =
                        AdmissionRepository::lock_and_bump_if_current(txn, admission_id, expected)
                            .await?;
                    // set_status validates the state-machine transition.
                    let enc =
                        EncounterRepository::set_status(txn, adm.encounter_id, target).await?;
                    AuditLogRepository::log(
                        txn,
                        "encounter",
                        enc.id,
                        audit_action,
                        None,
                        Some(serde_json::json!({
                            "admission_id": admission_id,
                            "status": format!("{:?}", target),
                        })),
                        &ctx_clone,
                    )
                    .await?;
                    let mut payload = serde_json::json!({
                        "admission_id": admission_id,
                        "encounter_id": enc.id,
                    });
                    if let Some(s) = source_for_txn.as_deref() {
                        payload["source"] = serde_json::Value::String(s.to_string());
                    }
                    OutboxRepository::publish(txn, event_type, &payload).await?;
                    Ok(enc)
                })
            })
            .await
            .map_err(Error::from)?;
        let mut in_mem = serde_json::json!({
            "admission_id": admission_id,
            "encounter_id": res.id,
        });
        if let Some(s) = source_owned.as_deref() {
            in_mem["source"] = serde_json::Value::String(s.to_string());
        }
        self.publisher
            .publish(DomainEvent::new(event_type, in_mem))
            .await
            .log_warn("in-process event");
        Ok(res)
    }
}
