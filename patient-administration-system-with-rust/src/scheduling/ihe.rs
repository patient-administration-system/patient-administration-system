//! Part of [`super`]: the service behind the IHE ITI Scheduling operations `$find`, `$hold` and `$book`
//! (NEW-10).
//!
//! A proposal is a free slot offered to one patient. `$find` records proposals in time order and nothing
//! else (no score, no reordering: NFR-11). `$hold` keeps a proposal's slot for `PAS_HOLD_MINUTES`: another
//! patient's booking of that slot, through any route, is refused while the hold runs, and `$find` leaves a
//! held slot out. `$book` books the slot through [`SchedulingService::book_inner`], the code behind
//! `POST /api/slots/{id}/book`, so the owner clash, patient overlap, group-session and interpreter rules
//! are the same. Holds are checked against the database clock when a booking is made, and against the
//! caller's `now` when one is made or listed, so a test passes real times.

use chrono::Duration;

use super::*;

/// How long a proposal from `$find` stays valid before it is held or booked.
const PROPOSAL_MINUTES: i64 = 60;

/// A slot proposed to a patient, held for them, or booked.
#[derive(Debug, Clone)]
pub struct Proposal {
    pub id: Uuid,
    pub slot_id: Uuid,
    pub person_id: Uuid,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    /// `proposed`, `held` or `booked`.
    pub status: String,
    pub expires_at: DateTime<Utc>,
    pub appointment_id: Option<Uuid>,
    /// The owner of the slot's schedule: `practitioner`, `room`, `bed` or `equipment`.
    pub owner_kind: String,
    pub owner_id: Uuid,
}

/// What `$find` asks for.
#[derive(Debug, Clone)]
pub struct FindQuery {
    pub patient: Uuid,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub practitioner: Option<Uuid>,
    /// A room or bed, as `Location/<id>`.
    pub location: Option<Uuid>,
    pub count: i64,
}

const PROPOSAL_SQL: &str = "SELECT p.id, p.slot_id, p.person_id, p.status, p.expires_at, p.appointment_id, \
        s.start_datetime, s.end_datetime, sc.owner_kind, sc.owner_id \
     FROM slot_proposals p JOIN slots s ON s.id = p.slot_id JOIN schedules sc ON sc.id = s.schedule_id";

fn proposal_of(r: &sea_orm::QueryResult) -> Result<Proposal> {
    Ok(Proposal {
        id: r.col("id")?,
        slot_id: r.col("slot_id")?,
        person_id: r.col("person_id")?,
        start: r.col("start_datetime")?,
        end: r.col("end_datetime")?,
        status: r.col("status")?,
        expires_at: r.col("expires_at")?,
        appointment_id: r.col("appointment_id")?,
        owner_kind: r.col("owner_kind")?,
        owner_id: r.col("owner_id")?,
    })
}

fn gone() -> Error {
    Error::not_found("that appointment is no longer available")
}

impl SchedulingService {
    /// Free one-to-one slots in the window that the patient can take, earliest first, each recorded as a
    /// proposal. Slots held for someone else and slots that overlap the patient's own appointments are left
    /// out; the owner clash is checked again at `$hold` and `$book`.
    pub async fn find_proposals(
        &self,
        q: &FindQuery,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<Vec<Proposal>> {
        if q.end <= q.start {
            return Err(Error::validation("end must be after start"));
        }
        if q.end - q.start > Duration::days(90) {
            return Err(Error::validation("the period can be at most 90 days"));
        }
        let from = q.start.max(now);
        let txn = self.db.begin().await.map_err(Error::Database)?;
        let rows = txn
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT s.id AS slot_id FROM slots s JOIN schedules sc ON sc.id = s.schedule_id \
                 WHERE s.status = 'free' AND sc.active AND s.capacity = 1 AND NOT s.ring_fenced \
                   AND s.start_datetime >= $1 AND s.start_datetime < $2 \
                   AND ($3::uuid IS NULL OR (sc.owner_kind = 'practitioner' AND sc.owner_id = $3)) \
                   AND ($4::uuid IS NULL OR (sc.owner_kind IN ('room','bed') AND sc.owner_id = $4)) \
                   AND NOT EXISTS (SELECT 1 FROM appointments a WHERE a.person_id = $5 \
                         AND a.deleted_at IS NULL AND a.status NOT IN ('cancelled','no_show') \
                         AND a.start_datetime < s.end_datetime AND a.end_datetime > s.start_datetime) \
                   AND NOT EXISTS (SELECT 1 FROM slot_proposals h WHERE h.slot_id = s.id \
                         AND h.status = 'held' AND h.expires_at > $6) \
                 ORDER BY s.start_datetime, s.id LIMIT $7",
                [
                    from.into(),
                    q.end.into(),
                    q.practitioner.into(),
                    q.location.into(),
                    q.patient.into(),
                    now.into(),
                    q.count.clamp(1, 50).into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        let mut ids = Vec::with_capacity(rows.len());
        for r in &rows {
            let slot_id: Uuid = r.col("slot_id")?;
            let id = Uuid::new_v4();
            txn.execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO slot_proposals (id, slot_id, person_id, status, expires_at) \
                 VALUES ($1, $2, $3, 'proposed', $4)",
                [
                    id.into(),
                    slot_id.into(),
                    q.patient.into(),
                    (now + Duration::minutes(PROPOSAL_MINUTES)).into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
            ids.push(id);
        }
        if !ids.is_empty() {
            AuditLogRepository::log(
                &txn,
                "slot_proposal",
                q.patient,
                "find",
                None,
                Some(serde_json::json!({ "proposals": ids, "start": q.start, "end": q.end })),
                ctx,
            )
            .await?;
        }
        txn.commit().await.map_err(Error::Database)?;
        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            out.push(self.proposal(id).await?);
        }
        Ok(out)
    }

    /// One proposal with its slot times and owner.
    pub async fn proposal(&self, id: Uuid) -> Result<Proposal> {
        let row = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!("{PROPOSAL_SQL} WHERE p.id = $1"),
                [id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(gone)?;
        proposal_of(&row)
    }

    /// Keep the proposal's slot for `ttl`. A hold already running on this proposal is returned as it is.
    /// Refused as no longer available when the proposal has lapsed, the slot is taken or held for someone
    /// else, or the room, clinician or equipment is busy at that time.
    pub async fn hold_proposal(
        &self,
        id: Uuid,
        now: DateTime<Utc>,
        ttl: Duration,
        ctx: &UserContext,
    ) -> Result<Proposal> {
        let ctx = ctx.clone();
        self.db
            .transaction::<_, (), Error>(|txn| {
                Box::pin(async move {
                    let p = txn
                        .query_one_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "SELECT slot_id, person_id, status, expires_at FROM slot_proposals \
                             WHERE id = $1 FOR UPDATE",
                            [id.into()],
                        ))
                        .await
                        .map_err(Error::Database)?
                        .ok_or_else(gone)?;
                    let slot_id: Uuid = p.col("slot_id")?;
                    let person_id: Uuid = p.col("person_id")?;
                    let status: String = p.col("status")?;
                    let expires_at: DateTime<Utc> = p.col("expires_at")?;
                    match status.as_str() {
                        "booked" => return Err(gone()),
                        "held" if expires_at > now => return Ok(()),
                        "proposed" if expires_at <= now => return Err(gone()),
                        _ => {}
                    }
                    let slot = SlotRepository::select_for_update(txn, slot_id)
                        .await?
                        .ok_or_else(gone)?;
                    if slot.status != SlotStatus::Free {
                        return Err(gone());
                    }
                    let other = txn
                        .query_one_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "SELECT 1 AS ok FROM slot_proposals WHERE slot_id = $1 AND id <> $2 \
                               AND status = 'held' AND expires_at > $3 LIMIT 1",
                            [slot_id.into(), id.into(), now.into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    if other.is_some() {
                        return Err(gone());
                    }
                    Self::check_owner_free(txn, &slot).await?;
                    let overlapping = AppointmentRepository::find_overlapping_for_patient(
                        txn,
                        person_id,
                        slot.start_datetime,
                        slot.end_datetime,
                    )
                    .await?;
                    if !overlapping.is_empty() {
                        return Err(Error::conflict(
                            "patient already has an overlapping appointment",
                        ));
                    }
                    let until = now + ttl;
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE slot_proposals SET status = 'held', held_at = $2, expires_at = $3 \
                         WHERE id = $1",
                        [id.into(), now.into(), until.into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    AuditLogRepository::log(
                        txn,
                        "slot_proposal",
                        id,
                        "hold",
                        None,
                        Some(serde_json::json!({
                            "slot_id": slot_id, "person_id": person_id, "until": until,
                        })),
                        &ctx,
                    )
                    .await?;
                    Ok(())
                })
            })
            .await
            .map_err(Error::from)?;
        self.proposal(id).await
    }

    /// Book the proposal's slot for its patient, with the rules of `POST /api/slots/{id}/book`. A lapsed
    /// proposal is refused; a hold that has run out does not stop the booking while the slot is still free.
    pub async fn book_proposal(
        &self,
        id: Uuid,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<(Proposal, Appointment)> {
        let ctx = ctx.clone();
        let appt = self
            .db
            .transaction::<_, Appointment, Error>(|txn| {
                Box::pin(async move {
                    let p = txn
                        .query_one_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "SELECT slot_id, person_id, status, expires_at FROM slot_proposals \
                             WHERE id = $1 FOR UPDATE",
                            [id.into()],
                        ))
                        .await
                        .map_err(Error::Database)?
                        .ok_or_else(gone)?;
                    let slot_id: Uuid = p.col("slot_id")?;
                    let person_id: Uuid = p.col("person_id")?;
                    let status: String = p.col("status")?;
                    let expires_at: DateTime<Utc> = p.col("expires_at")?;
                    match status.as_str() {
                        "booked" => return Err(Error::conflict("that appointment is already booked")),
                        "proposed" if expires_at <= now => return Err(gone()),
                        _ => {}
                    }
                    let appt = Self::book_inner(txn, slot_id, person_id, EntryLink::None, &ctx).await?;
                    txn.execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE slot_proposals SET status = 'booked', appointment_id = $2 WHERE id = $1",
                        [id.into(), appt.id.into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                    Ok(appt)
                })
            })
            .await
            .map_err(Error::from)?;
        self.publish_booked(&appt).await;
        Ok((self.proposal(id).await?, appt))
    }
}
