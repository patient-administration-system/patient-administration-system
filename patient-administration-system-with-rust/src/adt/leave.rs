//! Part of [`super`]: leave.

use super::*;

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
        self.transition_loa(
            admission_id,
            source,
            ctx,
            EncounterStatus::OnLeave,
            "start_leave",
            "EncounterLeaveStarted",
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
        self.transition_loa(
            admission_id,
            source,
            ctx,
            EncounterStatus::InProgress,
            "end_leave",
            "EncounterLeaveEnded",
        )
        .await
    }

    pub(super) async fn transition_loa(
        &self,
        admission_id: Uuid,
        source: Option<&str>,
        ctx: &UserContext,
        target: EncounterStatus,
        audit_action: &'static str,
        event_type: &'static str,
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
