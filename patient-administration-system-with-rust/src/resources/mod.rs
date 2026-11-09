//! resources — Bed/Ward operations service

use crate::db::sql::{Pool, Transact};
use std::sync::Arc;
use uuid::Uuid;

use crate::Result;
use crate::db::repositories::{
    audit::{AuditLogRepository, UserContext},
    bed::BedRepository,
    outbox::OutboxRepository,
};
use crate::error::LogErr;
use crate::models::facility::{Bed, BedStatus};
use crate::streaming::{DomainEvent, EventPublisher};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct WardOccupancy {
    pub ward_id: Uuid,
    pub total_beds: usize,
    pub available: usize,
    pub occupied: usize,
    pub cleaning: usize,
    pub reserved: usize,
    pub out_of_service: usize,
}

/// One bed on the ward overview (WPAS-4, FR-218). Nothing clinical: who, where, since when.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct BedLine {
    pub bed_id: Uuid,
    pub bed: String,
    pub room: String,
    pub status: BedStatus,
    /// The person in the bed, if anyone is.
    #[schema(required = true)]
    pub person_id: Option<Uuid>,
    #[schema(required = true)]
    pub encounter_id: Option<Uuid>,
    #[schema(required = true)]
    pub admission_id: Option<Uuid>,
    /// When the person was put in this bed.
    #[schema(required = true)]
    pub in_bed_since: Option<chrono::DateTime<chrono::Utc>>,
    #[schema(required = true)]
    pub admitted_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whole days since admission, counted in the service's time zone.
    #[schema(required = true)]
    pub days_in_hospital: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct WardOverview {
    pub ward_id: Uuid,
    pub occupancy: WardOccupancy,
    pub beds: Vec<BedLine>,
}

pub struct ResourcesService {
    db: Pool,
    publisher: Arc<dyn EventPublisher>,
}

impl ResourcesService {
    pub fn new(db: Pool, publisher: Arc<dyn EventPublisher>) -> Self {
        Self { db, publisher }
    }

    /// Every bed of a ward, by room and bed name, with who is in it (WPAS-4, FR-218).
    pub async fn ward_overview(&self, ward_id: Uuid) -> Result<WardOverview> {
        use crate::db::row::RowExt;
        use crate::db::sql::{Conn, Stmt};
        let occupancy = self.ward_occupancy(ward_id).await?;
        let tz = crate::inbound::messages::timezone();
        let today = chrono::Utc::now().with_timezone(&tz).date_naive();
        let rows = self
            .db
            .all(Stmt::new(
                "SELECT b.id AS bed_id, b.name AS bed, r.name AS room, b.status, e.person_id, ba.encounter_id, \
                   a.id AS admission_id, ba.assigned_at, a.admitted_at \
                 FROM beds b JOIN rooms r ON r.id = b.room_id \
                 LEFT JOIN bed_assignments ba ON ba.bed_id = b.id AND ba.released_at IS NULL \
                 LEFT JOIN encounters e ON e.id = ba.encounter_id \
                 LEFT JOIN admissions a ON a.encounter_id = ba.encounter_id \
                 WHERE r.ward_id = $1 ORDER BY r.name, b.name, b.id",
                [ward_id.into()],
            ))
            .await?;
        let beds = rows
            .iter()
            .map(|r| {
                let admitted_at: Option<chrono::DateTime<chrono::Utc>> = r.col("admitted_at")?;
                Ok(BedLine {
                    bed_id: r.col("bed_id")?,
                    bed: r.col("bed")?,
                    room: r.col("room")?,
                    status: crate::db::repositories::bed::bed_status_from_str(
                        &r.col::<String>("status")?,
                    )?,
                    person_id: r.col("person_id")?,
                    encounter_id: r.col("encounter_id")?,
                    admission_id: r.col("admission_id")?,
                    in_bed_since: r.col("assigned_at")?,
                    admitted_at,
                    days_in_hospital: admitted_at.map(|t| {
                        (today - t.with_timezone(&tz).date_naive())
                            .num_days()
                            .max(0)
                    }),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(WardOverview {
            ward_id,
            occupancy,
            beds,
        })
    }

    pub async fn ward_occupancy(&self, ward_id: Uuid) -> Result<WardOccupancy> {
        let beds = BedRepository::list_by_ward(&self.db, ward_id).await?;
        Ok(tally_occupancy(ward_id, &beds))
    }

    pub async fn set_bed_status(
        &self,
        bed_id: Uuid,
        new_status: BedStatus,
        ctx: &UserContext,
    ) -> Result<Bed> {
        self.set_bed_status_if_unchanged(bed_id, new_status, None, ctx)
            .await
    }

    /// [`Self::set_bed_status`] that refuses with `PreconditionFailed` when the bed has changed since the
    /// caller read it (OFH-10). The bed's row is locked first.
    pub async fn set_bed_status_if_unchanged(
        &self,
        bed_id: Uuid,
        new_status: BedStatus,
        expected: Option<chrono::DateTime<chrono::Utc>>,
        ctx: &UserContext,
    ) -> Result<Bed> {
        let ctx_clone = ctx.clone();
        let bed = self
            .db
            .in_txn::<_, Bed>(|txn| {
                Box::pin(async move {
                    let bed = BedRepository::update_status_if_current(
                        txn, bed_id, new_status, expected, true,
                    )
                    .await?;
                    AuditLogRepository::log(
                        txn,
                        "bed",
                        bed_id,
                        "set_status",
                        None,
                        Some(serde_json::json!({ "status": format!("{:?}", new_status) })),
                        &ctx_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "BedStatusChanged",
                        &serde_json::json!({
                            "bed_id": bed_id,
                            "status": format!("{:?}", new_status),
                        }),
                    )
                    .await?;
                    if new_status == BedStatus::OutOfService {
                        OutboxRepository::publish(
                            txn,
                            "BedRetired",
                            &serde_json::json!({
                                "bed_id": bed_id,
                                "bed_code": bed.code.clone(),
                            }),
                        )
                        .await?;
                    }
                    Ok(bed)
                })
            })
            .await?;
        self.publisher
            .publish(DomainEvent::new(
                "BedStatusChanged",
                serde_json::json!({ "bed_id": bed_id }),
            ))
            .await
            .log_warn("in-process event");
        Ok(bed)
    }
}

fn tally_occupancy(ward_id: Uuid, beds: &[Bed]) -> WardOccupancy {
    let mut occ = WardOccupancy {
        ward_id,
        total_beds: beds.len(),
        available: 0,
        occupied: 0,
        cleaning: 0,
        reserved: 0,
        out_of_service: 0,
    };
    for b in beds {
        match b.status {
            BedStatus::Available => occ.available += 1,
            BedStatus::Occupied => occ.occupied += 1,
            BedStatus::Cleaning => occ.cleaning += 1,
            BedStatus::Reserved => occ.reserved += 1,
            BedStatus::OutOfService => occ.out_of_service += 1,
        }
    }
    occ
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_bed(status: BedStatus) -> Bed {
        let mut b = Bed::new(Uuid::new_v4(), "B1".into(), "B1".into());
        b.status = status;
        b
    }

    #[test]
    fn test_tally_occupancy_mixed() {
        let ward_id = Uuid::new_v4();
        let beds = vec![
            make_bed(BedStatus::Available),
            make_bed(BedStatus::Available),
            make_bed(BedStatus::Occupied),
            make_bed(BedStatus::Cleaning),
            make_bed(BedStatus::OutOfService),
        ];
        let occ = tally_occupancy(ward_id, &beds);
        assert_eq!(occ.total_beds, 5);
        assert_eq!(occ.available, 2);
        assert_eq!(occ.occupied, 1);
        assert_eq!(occ.cleaning, 1);
        assert_eq!(occ.out_of_service, 1);
        assert_eq!(occ.reserved, 0);
    }
}
