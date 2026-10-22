//! Demo reference data (`pas-seed`, QUAL-36): the places a ward needs, and nothing about people.
//!
//! One facility, one ward, one room and three beds, found by their codes and created only if missing, so
//! running it twice changes nothing. People and staff are not seeded here because the PAS holds none: they
//! live in `person-service` and `worker-service`, and an admission refers to a person by the id that service
//! gave them. Create people there, then use their ids with the PAS.

use sea_orm::{DatabaseConnection, TransactionTrait};
use uuid::Uuid;

use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::repositories::bed::BedRepository;
use crate::db::repositories::facility::{FacilityRepository, RoomRepository, WardRepository};
use crate::models::Address;
use crate::models::facility::{Bed, Facility, Room, Ward};
use crate::{Error, Result};

pub const FACILITY_CODE: &str = "DEMO";
pub const WARD_CODE: &str = "WARD-A";
pub const ROOM_CODE: &str = "ROOM-1";
pub const BED_CODES: [&str; 3] = ["BED-1", "BED-2", "BED-3"];

/// What the seed found or made.
#[derive(Debug, Clone)]
pub struct Seeded {
    pub facility_id: Uuid,
    pub ward_id: Uuid,
    pub room_id: Uuid,
    pub bed_ids: Vec<Uuid>,
    /// How many records this run created (0 when everything was already there).
    pub created: usize,
}

/// Create the demo facility, ward, room and beds that do not exist yet, in one transaction.
pub async fn run(db: &DatabaseConnection) -> Result<Seeded> {
    db.transaction::<_, Seeded, Error>(|txn| {
        Box::pin(async move {
            let ctx = UserContext::system();
            let mut created = 0usize;

            let facility = match FacilityRepository::list(txn)
                .await?
                .into_iter()
                .find(|f| f.code == FACILITY_CODE)
            {
                Some(f) => f,
                None => {
                    let f = FacilityRepository::create(
                        txn,
                        &Facility::new(
                            "Demo Hospital".into(),
                            FACILITY_CODE.into(),
                            Address::default(),
                        ),
                    )
                    .await?;
                    AuditLogRepository::log(txn, "facility", f.id, "seed", None, None, &ctx)
                        .await?;
                    created += 1;
                    f
                }
            };
            let ward = match WardRepository::list_by_facility(txn, facility.id)
                .await?
                .into_iter()
                .find(|w| w.code == WARD_CODE)
            {
                Some(w) => w,
                None => {
                    let w = WardRepository::create(
                        txn,
                        &Ward::new(facility.id, "Ward A".into(), WARD_CODE.into(), 3),
                    )
                    .await?;
                    AuditLogRepository::log(txn, "ward", w.id, "seed", None, None, &ctx).await?;
                    created += 1;
                    w
                }
            };
            let room = match RoomRepository::find_by_code(txn, ROOM_CODE).await? {
                Some(r) => r,
                None => {
                    let r = RoomRepository::create(
                        txn,
                        &Room::new(ward.id, "Room 1".into(), ROOM_CODE.into()),
                    )
                    .await?;
                    AuditLogRepository::log(txn, "room", r.id, "seed", None, None, &ctx).await?;
                    created += 1;
                    r
                }
            };
            let mut bed_ids = Vec::new();
            for (i, code) in BED_CODES.iter().enumerate() {
                let bed = match BedRepository::find_by_code(txn, code).await? {
                    Some(b) => b,
                    None => {
                        let b = BedRepository::create(
                            txn,
                            &Bed::new(room.id, format!("Bed {}", i + 1), (*code).into()),
                        )
                        .await?;
                        AuditLogRepository::log(txn, "bed", b.id, "seed", None, None, &ctx).await?;
                        created += 1;
                        b
                    }
                };
                bed_ids.push(bed.id);
            }
            Ok(Seeded {
                facility_id: facility.id,
                ward_id: ward.id,
                room_id: room.id,
                bed_ids,
                created,
            })
        })
    })
    .await
    .map_err(Error::from)
}
