//! Patient-initiated follow-up, PIFU (NEW-3).
//!
//! A clinician opens an episode for a patient: a service, a window, and the conditions in words the patient
//! can follow ("contact us if the pain gets worse or you cannot walk further than..."). Then:
//!
//! - a request **inside** the window becomes a booking task for a person to act on;
//! - a request **after** the window goes to a person to decide, never to a booking;
//! - when the window ends, the episode is listed for a person's review (discharge or extend). Nothing here
//!   discharges a patient by itself: the review list is the episodes past their end that nobody resolved;
//! - a service on the urgent follow-up list (CAP-7) is excluded unless a named clinician overrides it.
//!
//! PIFU informs people and creates work; it never changes a waiting-list order or an offer (NFR-11).

use crate::db::row::RowExt;
use crate::db::sql::Transact;
use crate::db::sql::{Conn, Row, Stmt};
use chrono::{DateTime, Duration, Utc};
use sea_orm::DatabaseConnection;
use serde::Serialize;
use uuid::Uuid;

use crate::analytics::{Cell, suppress};
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::validation::named_person;
use crate::{Error, Result};

crate::string_enum! {
    /// How a request reached the PAS.
    pub enum RequestChannel {
        Staff = "staff",
        Phone = "phone",
        Text = "text",
        ReplyLink = "reply_link",
        Portal = "portal",
    }
}

crate::string_enum! {
    /// Whether an episode is still open.
    pub enum EpisodeStatus {
        Active = "active",
        Closed = "closed",
    }
}

/// The longest window an episode may be opened or extended for.
pub const MAX_WINDOW_DAYS: i64 = 730;

/// Where a request goes: a booking task inside the window, a person's decision after it.
pub fn route_for(now: DateTime<Utc>, expires_at: DateTime<Utc>) -> &'static str {
    if now < expires_at {
        "booking"
    } else {
        "person"
    }
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
#[schema(as = pifu::Episode)]
pub struct Episode {
    pub id: Uuid,
    pub person_id: Uuid,
    pub service_type: String,
    pub conditions: String,
    pub opened_by: String,
    pub opened_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[schema(required = true)]
    pub urgent_override_by: Option<String>,
    pub extended_count: i32,
    pub status: EpisodeStatus,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
#[schema(as = pifu::Request)]
pub struct Request {
    pub id: Uuid,
    pub episode_id: Uuid,
    pub person_id: Uuid,
    pub service_type: String,
    pub requested_at: DateTime<Utc>,
    pub channel: RequestChannel,
    /// `booking` or `person`.
    pub route: String,
}

/// What a clinician supplies to open an episode.
#[derive(Debug, Clone, Copy)]
pub struct NewEpisode<'a> {
    pub service_type: &'a str,
    pub conditions: &'a str,
    pub expires_at: DateTime<Utc>,
    pub opened_by: &'a str,
    /// Required when the service is on the urgent follow-up list.
    pub urgent_override_by: Option<&'a str>,
}

/// What the review decides.
#[derive(Debug, Clone, Copy)]
pub enum Decision {
    Discharge,
    Extend(DateTime<Utc>),
}

const EP_COLS: &str = "id, person_id, service_type, conditions, opened_by, opened_at, expires_at, \
    urgent_override_by, extended_count, status";

fn episode(r: &Row) -> Result<Episode> {
    Ok(Episode {
        id: r.col("id")?,
        person_id: r.col("person_id")?,
        service_type: r.col("service_type")?,
        conditions: r.col("conditions")?,
        opened_by: r.col("opened_by")?,
        opened_at: r.col("opened_at")?,
        expires_at: r.col("expires_at")?,
        urgent_override_by: r.col("urgent_override_by")?,
        extended_count: r.col("extended_count")?,
        status: r.col::<String>("status")?.parse()?,
    })
}

/// The uptake report: counts only, no identifiers, small cells withheld.
#[derive(Debug, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct UptakeReport {
    pub k: i64,
    pub cells: Vec<Cell>,
}

pub struct PifuService {
    db: DatabaseConnection,
}

impl PifuService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// A clinician opens an episode. Audited. A service on the urgent follow-up list needs a named override.
    pub async fn open(
        &self,
        person_id: Uuid,
        new: &NewEpisode<'_>,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<Episode> {
        let by = named_person(
            new.opened_by,
            "opened_by: the clinician opening the episode is required",
        )?;
        let service = new.service_type.trim();
        if service.is_empty() || service.chars().count() > 100 {
            return Err(Error::validation(
                "service_type is required (at most 100 characters)",
            ));
        }
        let conditions = new.conditions.trim();
        if !(10..=1000).contains(&conditions.chars().count())
            || conditions.chars().any(|c| c.is_control() && c != '\n')
        {
            return Err(Error::validation(
                "conditions: write the conditions for the patient, 10 to 1000 characters",
            ));
        }
        check_window(now, new.expires_at)?;
        let override_by = new
            .urgent_override_by
            .map(|o| named_person(o, "urgent_override_by must be a name"))
            .transpose()?;
        let id = Uuid::new_v4();
        let txn = self.db.begin_txn().await?;
        let urgent = txn
            .one(Stmt::new(
                "SELECT 1 AS x FROM follow_up_policies WHERE service_type = $1",
                [service.into()],
            ))
            .await?
            .is_some();
        if urgent && override_by.is_none() {
            return Err(Error::validation(
                "this service is on the urgent follow-up list: a named clinician must override (urgent_override_by)",
            ));
        }
        let row = txn
            .one(Stmt::new(format!(
                    "INSERT INTO pifu_episodes (id, person_id, service_type, conditions, opened_by, opened_at, expires_at, urgent_override_by) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                     ON CONFLICT (person_id, service_type) WHERE status = 'active' DO NOTHING RETURNING {EP_COLS}"
                ),
                [
                    id.into(),
                    person_id.into(),
                    service.into(),
                    conditions.into(),
                    by.into(),
                    now.into(),
                    new.expires_at.into(),
                    override_by.into(),
                ],
            ))
.await?;
        let Some(row) = row else {
            return Err(Error::conflict(format!(
                "the patient already has a live PIFU episode for {service}"
            )));
        };
        AuditLogRepository::log(
            &txn,
            "pifu_episode",
            id,
            "open",
            None,
            Some(serde_json::json!({ "person_id": person_id, "service_type": service, "opened_by": by, "expires_at": new.expires_at })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        episode(&row)
    }

    /// A patient's request for follow-up. Inside the window it is a booking task; after it, or when the
    /// episode was closed, it goes to a person. A second request while one is open returns the open one.
    pub async fn request(
        &self,
        episode_id: Uuid,
        channel: &str,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<Request> {
        let channel: RequestChannel = channel.parse().map_err(|_| {
            Error::validation(format!(
                "channel must be one of {}",
                RequestChannel::NAMES.join(", ")
            ))
        })?;
        let txn = self.db.begin_txn().await?;
        let ep = txn
            .one(Stmt::new(
                format!("SELECT {EP_COLS} FROM pifu_episodes WHERE id = $1 FOR UPDATE"),
                [episode_id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("PIFU episode {episode_id}")))?;
        let ep = episode(&ep)?;
        let route = if ep.status == EpisodeStatus::Active {
            route_for(now, ep.expires_at)
        } else {
            "person"
        };
        let id = Uuid::new_v4();
        let inserted = txn
            .run(Stmt::new("INSERT INTO pifu_requests (id, episode_id, person_id, requested_at, channel, route) \
                 VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT (episode_id) WHERE handled_at IS NULL DO NOTHING",
                [
                    id.into(),
                    episode_id.into(),
                    ep.person_id.into(),
                    now.into(),
                    channel.as_str().into(),
                    route.into(),
                ],
            ))
.await?;
        if inserted.rows_affected() == 1 {
            AuditLogRepository::log(
                &txn,
                "pifu_request",
                id,
                "request",
                None,
                Some(serde_json::json!({ "episode_id": episode_id, "channel": channel.as_str(), "route": route })),
                ctx,
            )
            .await?;
        }
        let open = txn
            .one(Stmt::new(
                "SELECT id, episode_id, person_id, requested_at, channel, route FROM pifu_requests \
                 WHERE episode_id = $1 AND handled_at IS NULL",
                [episode_id.into()],
            ))
            .await?
            .ok_or_else(|| {
                Error::internal("an open PIFU request was not found after recording one")
            })?;
        txn.commit().await.map_err(Error::Database)?;
        Ok(Request {
            id: open.col("id")?,
            episode_id,
            person_id: ep.person_id,
            service_type: ep.service_type,
            requested_at: open.col("requested_at")?,
            channel: open.col::<String>("channel")?.parse()?,
            route: open.col("route")?,
        })
    }

    /// Open requests, oldest first, for the booking team (`booking`) or for a person's decision (`person`).
    pub async fn open_requests(&self, route: &str, fetch: i64) -> Result<Vec<Request>> {
        if !["booking", "person"].contains(&route) {
            return Err(Error::validation("route must be booking or person"));
        }
        let rows = self
            .db
            .all(Stmt::new("SELECT r.id, r.episode_id, r.person_id, e.service_type, r.requested_at, r.channel, r.route \
                 FROM pifu_requests r JOIN pifu_episodes e ON e.id = r.episode_id \
                 WHERE r.handled_at IS NULL AND r.route = $1 ORDER BY r.requested_at, r.id LIMIT $2",
                [route.into(), crate::paging::fetch(fetch).into()],
            ))
.await?;
        rows.iter()
            .map(|r| {
                Ok(Request {
                    id: r.col("id")?,
                    episode_id: r.col("episode_id")?,
                    person_id: r.col("person_id")?,
                    service_type: r.col("service_type")?,
                    requested_at: r.col("requested_at")?,
                    channel: r.col::<String>("channel")?.parse()?,
                    route: r.col("route")?,
                })
            })
            .collect()
    }

    /// A named person has dealt with a request (booked, or decided), once.
    pub async fn handle(
        &self,
        request_id: Uuid,
        handled_by: &str,
        appointment_id: Option<Uuid>,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = named_person(
            handled_by,
            "handled_by: the person handling the request is required",
        )?;
        let txn = self.db.begin_txn().await?;
        let done = txn
            .run(Stmt::new(
                "UPDATE pifu_requests SET handled_by = $2, handled_at = $3, appointment_id = $4 \
                 WHERE id = $1 AND handled_at IS NULL",
                [
                    request_id.into(),
                    by.into(),
                    now.into(),
                    appointment_id.into(),
                ],
            ))
            .await?;
        if done.rows_affected() != 1 {
            return Err(Error::not_found(format!("open PIFU request {request_id}")));
        }
        AuditLogRepository::log(
            &txn,
            "pifu_request",
            request_id,
            "handle",
            None,
            Some(serde_json::json!({ "handled_by": by, "appointment_id": appointment_id })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }

    /// Episodes whose window has ended and nobody has reviewed: the review task. Oldest expiry first.
    pub async fn due_reviews(&self, now: DateTime<Utc>, fetch: i64) -> Result<Vec<Episode>> {
        let rows = self
            .db
            .all(Stmt::new(format!(
                    "SELECT {EP_COLS} FROM pifu_episodes WHERE status = 'active' AND expires_at <= $1 \
                     ORDER BY expires_at, id LIMIT $2"
                ),
                [now.into(), crate::paging::fetch(fetch).into()],
            ))
.await?;
        rows.iter().map(episode).collect()
    }

    /// A named person reviews an episode whose window has ended: discharge (closes it) or extend (a new
    /// end date). A review with an unhandled request open is refused: the request is dealt with first.
    pub async fn review(
        &self,
        episode_id: Uuid,
        decision: Decision,
        by: &str,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<()> {
        let by = named_person(
            by,
            "reviewed_by: the person reviewing the episode is required",
        )?;
        if let Decision::Extend(until) = decision {
            check_window(now, until)?;
        }
        let txn = self.db.begin_txn().await?;
        let ep = txn
            .one(Stmt::new(
                format!("SELECT {EP_COLS} FROM pifu_episodes WHERE id = $1 FOR UPDATE"),
                [episode_id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("PIFU episode {episode_id}")))?;
        let ep = episode(&ep)?;
        if ep.status != EpisodeStatus::Active {
            return Err(Error::conflict("the episode is already closed"));
        }
        if ep.expires_at > now {
            return Err(Error::conflict(
                "the window has not ended: the review is due when it does",
            ));
        }
        let open_request = txn
            .one(Stmt::new(
                "SELECT 1 AS x FROM pifu_requests WHERE episode_id = $1 AND handled_at IS NULL",
                [episode_id.into()],
            ))
            .await?
            .is_some();
        if open_request {
            return Err(Error::conflict(
                "the patient has an open request: deal with it before reviewing",
            ));
        }
        let action = match decision {
            Decision::Discharge => {
                txn.run(Stmt::new("UPDATE pifu_episodes SET status = 'closed', closed_by = $2, closed_at = $3 WHERE id = $1",
                    [episode_id.into(), by.into(), now.into()],
                ))
.await?;
                "discharge"
            }
            Decision::Extend(until) => {
                txn.run(Stmt::new("UPDATE pifu_episodes SET expires_at = $2, extended_count = extended_count + 1 WHERE id = $1",
                    [episode_id.into(), until.into()],
                ))
.await?;
                "extend"
            }
        };
        AuditLogRepository::log(
            &txn,
            "pifu_episode",
            episode_id,
            action,
            None,
            Some(serde_json::json!({ "reviewed_by": by })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)
    }

    /// Uptake, apart from discharges: how many episodes opened, requests inside and after the window,
    /// discharges and extensions at review. Counts only; cells under `k` withheld.
    pub async fn uptake(&self, k: i64) -> Result<UptakeReport> {
        let row = self
            .db
            .one(Stmt::text("SELECT \
                   (SELECT count(*) FROM pifu_episodes)::bigint AS opened, \
                   (SELECT count(*) FROM pifu_requests WHERE route = 'booking')::bigint AS in_window, \
                   (SELECT count(*) FROM pifu_requests WHERE route = 'person')::bigint AS after_window, \
                   (SELECT count(*) FROM pifu_episodes WHERE status = 'closed')::bigint AS discharged, \
                   (SELECT COALESCE(sum(extended_count), 0) FROM pifu_episodes)::bigint AS extended",
            ))
.await?
            .ok_or_else(|| Error::internal("uptake counts missing"))?;
        let mut rows = Vec::new();
        for (measure, col) in [
            ("episodes_opened", "opened"),
            ("requests_in_window", "in_window"),
            ("requests_after_window", "after_window"),
            ("discharged_at_review", "discharged"),
            ("extended_at_review", "extended"),
        ] {
            let n: i64 = row.col(col)?;
            rows.push((vec![("measure".to_string(), measure.to_string())], n));
        }
        Ok(UptakeReport {
            k,
            cells: suppress(rows, k),
        })
    }
}

fn check_window(now: DateTime<Utc>, until: DateTime<Utc>) -> Result<()> {
    if until <= now {
        return Err(Error::validation("the window must end in the future"));
    }
    if until > now + Duration::days(MAX_WINDOW_DAYS) {
        return Err(Error::validation(format!(
            "the window may be at most {MAX_WINDOW_DAYS} days"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_the_window_is_a_booking_and_at_or_after_its_end_is_a_person() {
        let end = Utc::now() + Duration::days(10);
        assert_eq!(route_for(end - Duration::seconds(1), end), "booking");
        assert_eq!(route_for(end, end), "person");
        assert_eq!(route_for(end + Duration::days(1), end), "person");
    }

    #[test]
    fn a_window_must_end_in_the_future_and_within_two_years() {
        let now = Utc::now();
        assert!(check_window(now, now - Duration::days(1)).is_err());
        assert!(check_window(now, now + Duration::days(MAX_WINDOW_DAYS + 1)).is_err());
        assert!(check_window(now, now + Duration::days(180)).is_ok());
    }
}
