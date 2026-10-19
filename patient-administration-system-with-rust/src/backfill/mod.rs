//! Closing the backfill loop (PILOT-8).
//!
//! When a slot is released, the next standby patient is offered it, for a limited time. They accept or
//! decline with a single-use link; if they decline, or the offer runs out, the next patient is offered it.
//!
//! - **Order is time on standby and nothing else.** Candidates come from [`StandbyService::candidates`],
//!   which orders by when they joined the list. No score, flag or guess about who is likely to accept takes
//!   part (NFR-11). A patient is offered a given release at most once.
//! - **One live offer per release.** The next offer is made only when the last one is answered or expires.
//! - **Time-limited.** An offer lasts `PAS_OFFER_MINUTES` (default 30) and never past the slot's start.
//! - **Accepting books the slot and takes them off standby in one transaction**, or neither happens. A
//!   slot that someone else has taken meanwhile is `missed`, not double-booked.
//! - **Safe links.** The token is opaque (244 bits), only its hash is stored, a GET reads and never acts,
//!   and the link works once.
//! - **Consent first.** The offer goes through the same choke point as every message: opt-outs and quiet
//!   hours apply. If a patient cannot be messaged, the offer simply runs out and the next patient is tried.
//!   Nothing is sent unless `PAS_BACKFILL_OFFERS=true`.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::communication::CommunicationService;
use crate::communication::consent;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::inbound::messages;
use crate::models::communication::{DeliveryChannel, LetterStatus};
use crate::standby::{CandidateFilter, StandbyService};
use crate::{Error, Result};

pub const TEMPLATE_NAME: &str = "slot_offer";

fn ttl() -> Duration {
    let m = std::env::var("PAS_OFFER_MINUTES")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|m| *m > 0)
        .unwrap_or(30);
    Duration::minutes(m)
}

pub fn enabled() -> bool {
    std::env::var("PAS_BACKFILL_OFFERS").as_deref() == Ok("true")
}

fn new_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

pub fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct Offer {
    pub id: Uuid,
    pub release_id: Uuid,
    pub slot_id: Uuid,
    pub person_id: Uuid,
    pub expires_at: DateTime<Utc>,
}

/// What the offer page may show: the time and nothing about the service or anyone else.
#[derive(Debug, Clone, Serialize)]
pub struct OfferInfo {
    /// `open`, `used`, `expired` or `gone` (the slot was taken).
    pub state: &'static str,
    pub starts_at: Option<DateTime<Utc>>,
    pub time_zone: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub actions: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OfferOutcome {
    /// `accepted`, `declined` or `gone`.
    pub status: &'static str,
    pub starts_at: Option<DateTime<Utc>>,
    pub time_zone: String,
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct TickReport {
    pub expired: usize,
    pub offered: usize,
}

pub struct BackfillService {
    db: DatabaseConnection,
    standby: Arc<StandbyService>,
    communication: Arc<CommunicationService>,
    link_base: Option<String>,
}

struct Row {
    id: Uuid,
    release_id: Uuid,
    slot_id: Uuid,
    standby_entry_id: Uuid,
    status: String,
    expires_at: DateTime<Utc>,
}

impl BackfillService {
    pub fn new(
        db: DatabaseConnection,
        standby: Arc<StandbyService>,
        communication: Arc<CommunicationService>,
    ) -> Self {
        Self {
            db,
            standby,
            communication,
            link_base: std::env::var("PAS_LINK_BASE_URL")
                .ok()
                .filter(|s| !s.is_empty()),
        }
    }

    pub fn with_link_base(mut self, base: Option<String>) -> Self {
        self.link_base = base;
        self
    }

    pub fn derive(&self) -> Self {
        Self {
            db: self.db.clone(),
            standby: self.standby.clone(),
            communication: self.communication.clone(),
            link_base: self.link_base.clone(),
        }
    }

    pub fn with_communication(mut self, c: Arc<CommunicationService>) -> Self {
        self.communication = c;
        self
    }

    async fn row_by_hash(&self, h: &str) -> Result<Row> {
        let r = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id, release_id, slot_id, standby_entry_id, status, expires_at \
                 FROM slot_offers WHERE token_hash = $1",
                [h.into()],
            ))
            .await
            .map_err(Error::Database)?
            // The same answer for a malformed and an unknown token.
            .ok_or_else(|| Error::not_found("this link is not valid"))?;
        Ok(Row {
            id: r.try_get("", "id").map_err(Error::Database)?,
            release_id: r.try_get("", "release_id").map_err(Error::Database)?,
            slot_id: r.try_get("", "slot_id").map_err(Error::Database)?,
            standby_entry_id: r.try_get("", "standby_entry_id").map_err(Error::Database)?,
            status: r.try_get("", "status").map_err(Error::Database)?,
            expires_at: r.try_get("", "expires_at").map_err(Error::Database)?,
        })
    }

    async fn slot_start(&self, slot_id: Uuid) -> Result<Option<DateTime<Utc>>> {
        self.db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT start_datetime FROM slots WHERE id = $1",
                [slot_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .map(|r| r.try_get("", "start_datetime").map_err(Error::Database))
            .transpose()
    }

    /// Offer an open release to the next standby patient, if there is one and no offer is live. Returns the
    /// offer and its token (shown once, for the message).
    pub async fn offer_next(&self, release_id: Uuid) -> Result<Option<(Offer, String)>> {
        let live = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT 1 AS ok FROM slot_offers WHERE release_id = $1 AND status = 'open'",
                [release_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if live.is_some() {
            return Ok(None);
        }
        let rel = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT slot_id, status, start_datetime FROM slot_releases WHERE id = $1",
                [release_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("slot release {release_id}")))?;
        let status: String = rel.try_get("", "status").map_err(Error::Database)?;
        let start: DateTime<Utc> = rel.try_get("", "start_datetime").map_err(Error::Database)?;
        let slot_id: Uuid = rel.try_get("", "slot_id").map_err(Error::Database)?;
        let now = Utc::now();
        if status != "open" || start <= now {
            return Ok(None);
        }
        // Never offered this release before, in time-on-standby order and nothing else.
        let already: Vec<Uuid> = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT person_id FROM slot_offers WHERE release_id = $1",
                [release_id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .iter()
            .filter_map(|r| r.try_get("", "person_id").ok())
            .collect();
        let candidates = self
            .standby
            .candidates(release_id, &CandidateFilter::default())
            .await?;
        let Some(next) = candidates
            .into_iter()
            .find(|c| !already.contains(&c.person_id))
        else {
            return Ok(None);
        };
        let token = new_token();
        let id = Uuid::new_v4();
        let expires_at = (now + ttl()).min(start);
        let made = self
            .db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO slot_offers (id, release_id, slot_id, standby_entry_id, person_id, token_hash, expires_at) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT DO NOTHING",
                [
                    id.into(),
                    release_id.into(),
                    slot_id.into(),
                    next.id.into(),
                    next.person_id.into(),
                    hash(&token).into(),
                    expires_at.into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        if made.rows_affected() == 0 {
            return Ok(None); // another scan made an offer first
        }
        AuditLogRepository::log_with(
            &self.db,
            "slot_release",
            release_id,
            "offered",
            None,
            Some(serde_json::json!({ "offer_id": id, "expires_at": expires_at })),
            Some("backfill_offer"),
            None,
            &UserContext::system(),
        )
        .await?;
        let offer = Offer {
            id,
            release_id,
            slot_id,
            person_id: next.person_id,
            expires_at,
        };
        Ok(Some((offer, token)))
    }

    /// Tell the patient, through the same choke point as every message. Returns whether one was sent.
    pub async fn notify(&self, offer: &Offer, token: &str) -> Result<bool> {
        let Some(base) = &self.link_base else {
            return Ok(false); // no address to put in a link
        };
        let Some(start) = self.slot_start(offer.slot_id).await? else {
            return Ok(false);
        };
        let lang = crate::inbound::messages::Lang::from_preference(
            crate::contact::preferred_language(&self.db, offer.person_id)
                .await?
                .as_deref(),
        );
        let templates = crate::communication::reminders::ReminderService::new(
            self.db.clone(),
            self.communication.clone(),
        )
        .ensure_templates_named(TEMPLATE_NAME)
        .await?;
        let wanted = crate::communication::templates::choose(
            &templates,
            Some(lang.code()),
            &crate::communication::templates::required_locales(),
        );
        let Some(template) = wanted else {
            return Ok(false);
        };
        let ctx = UserContext::system();
        let quiet = consent::QuietHours::from_env()?;
        let extra = serde_json::json!({
            "offer": {
                "when": messages::when(start, lang, messages::timezone()),
                "minutes": (offer.expires_at - Utc::now()).num_minutes().max(1),
            },
            "offer_url": format!("{base}/o/{token}"),
        });
        for channel in [
            DeliveryChannel::Sms,
            DeliveryChannel::WhatsApp,
            DeliveryChannel::AppleMessages,
            DeliveryChannel::Email,
        ] {
            if !template.channels.contains(&channel) || !self.communication.channel_ready(channel) {
                continue;
            }
            if consent::is_intrusive(channel)
                && quiet.is_some_and(|q| q.contains(Utc::now(), messages::timezone()))
            {
                // A short-lived offer is not worth waking anyone for; it runs out and the next is tried.
                continue;
            }
            match self
                .communication
                .generate_letter_for(
                    template.id,
                    offer.person_id,
                    None,
                    channel,
                    extra.clone(),
                    "slot_offer",
                    Some(offer.id.to_string()),
                    &ctx,
                )
                .await
            {
                Ok(l) if l.status == LetterStatus::Sent => {
                    self.db
                        .execute(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE slot_offers SET message_sent = true WHERE id = $1",
                            [offer.id.into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    return Ok(true);
                }
                Ok(_) => continue,
                Err(e) => {
                    tracing::info!(target: "pas::backfill", offer_id = %offer.id, "not messaged on {channel:?}: {e}");
                    continue;
                }
            }
        }
        Ok(false)
    }

    /// Offer and notify. Used by the release hook and the scheduler.
    pub async fn offer_and_notify(&self, release_id: Uuid) -> Result<bool> {
        match self.offer_next(release_id).await? {
            Some((offer, token)) => {
                if let Err(e) = self.notify(&offer, &token).await {
                    tracing::warn!(target: "pas::backfill", "could not notify: {e}");
                }
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// A cancellation freed a slot: if offers are switched on, offer it.
    pub async fn on_cancelled(&self, appointment_id: Uuid) -> Result<()> {
        if !enabled() {
            return Ok(());
        }
        let rel = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id FROM slot_releases WHERE appointment_id = $1 AND status = 'open' \
                 ORDER BY released_at DESC LIMIT 1",
                [appointment_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if let Some(r) = rel {
            let id: Uuid = r.try_get("", "id").map_err(Error::Database)?;
            self.offer_and_notify(id).await?;
        }
        Ok(())
    }

    /// Expire offers past their time and move each release on to the next patient.
    pub async fn tick(&self, now: DateTime<Utc>) -> Result<TickReport> {
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE slot_offers SET status = 'expired', answered_at = $1 \
                 WHERE status = 'open' AND expires_at <= $1 RETURNING release_id",
                [now.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let mut report = TickReport {
            expired: rows.len(),
            offered: 0,
        };
        for r in rows {
            let rid: Uuid = r.try_get("", "release_id").map_err(Error::Database)?;
            if self.offer_and_notify(rid).await? {
                report.offered += 1;
            }
        }
        Ok(report)
    }

    /// Read-only. Never changes anything, however often or by whom it is called.
    pub async fn info(&self, token: &str) -> Result<OfferInfo> {
        let row = self.row_by_hash(&hash(token)).await?;
        let start = self.slot_start(row.slot_id).await?;
        let tz = messages::timezone().name().to_string();
        let state = match row.status.as_str() {
            "open" if row.expires_at <= Utc::now() => "expired",
            "open" => "open",
            "missed" => "gone",
            "expired" => "expired",
            _ => "used",
        };
        Ok(OfferInfo {
            state,
            starts_at: if state == "open" { start } else { None },
            time_zone: tz,
            expires_at: (state == "open").then_some(row.expires_at),
            actions: if state == "open" {
                vec!["accept", "decline"]
            } else {
                vec![]
            },
        })
    }

    /// Accept: book the slot and take the patient off standby, atomically. Claims the offer first, so two
    /// taps act once; gives it back if the booking cannot be made.
    pub async fn accept(&self, token: &str, ctx: &UserContext) -> Result<OfferOutcome> {
        let h = hash(token);
        let row = self.row_by_hash(&h).await?;
        let tz = messages::timezone().name().to_string();
        if row.status != "open" {
            return Err(Error::conflict("this offer has already been answered"));
        }
        if row.expires_at <= Utc::now() {
            return Err(Error::validation("this offer has expired"));
        }
        let claimed = self
            .db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE slot_offers SET status = 'accepted', answered_at = now() \
                 WHERE id = $1 AND status = 'open' AND expires_at > now()",
                [row.id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if claimed.rows_affected() == 0 {
            return Err(Error::conflict("this offer has already been answered"));
        }
        match self
            .standby
            .book_from_standby(row.standby_entry_id, row.slot_id, ctx)
            .await
        {
            Ok(appt) => {
                self.db
                    .execute(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE slot_offers SET appointment_id = $2 WHERE id = $1",
                        [row.id.into(), appt.id.into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                Ok(OfferOutcome {
                    status: "accepted",
                    starts_at: Some(appt.start_datetime),
                    time_zone: tz,
                })
            }
            // The slot went another way, or the patient is no longer on standby or has a clash: this
            // offer is over, and the next patient is tried.
            Err(Error::Conflict(_)) | Err(Error::NotFound(_)) => {
                self.db
                    .execute(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE slot_offers SET status = 'missed' WHERE id = $1",
                        [row.id.into()],
                    ))
                    .await
                    .map_err(Error::Database)?;
                let _ = self.offer_and_notify(row.release_id).await;
                Ok(OfferOutcome {
                    status: "gone",
                    starts_at: None,
                    time_zone: tz,
                })
            }
            Err(e) => {
                // Unexpected: give the offer back so the patient can try again.
                let _ = self
                    .db
                    .execute(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE slot_offers SET status = 'open', answered_at = NULL WHERE id = $1",
                        [row.id.into()],
                    ))
                    .await;
                Err(e)
            }
        }
    }

    /// Decline: the offer is over and the next patient is offered the slot.
    pub async fn decline(&self, token: &str) -> Result<OfferOutcome> {
        let row = self.row_by_hash(&hash(token)).await?;
        let tz = messages::timezone().name().to_string();
        let r = self
            .db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE slot_offers SET status = 'declined', answered_at = now() \
                 WHERE id = $1 AND status = 'open' AND expires_at > now()",
                [row.id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if r.rows_affected() == 0 {
            return Err(Error::conflict(
                "this offer has already been answered or has expired",
            ));
        }
        let _ = self.offer_and_notify(row.release_id).await;
        Ok(OfferOutcome {
            status: "declined",
            starts_at: None,
            time_zone: tz,
        })
    }

    /// Offers for a release, oldest first (for staff).
    pub async fn offers_for_release(&self, release_id: Uuid) -> Result<Vec<serde_json::Value>> {
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id, status, offered_at, expires_at, message_sent FROM slot_offers \
                 WHERE release_id = $1 ORDER BY offered_at",
                [release_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                Ok(serde_json::json!({
                    "id": r.try_get::<Uuid>("", "id").map_err(Error::Database)?,
                    "status": r.try_get::<String>("", "status").map_err(Error::Database)?,
                    "offered_at": r.try_get::<DateTime<Utc>>("", "offered_at").map_err(Error::Database)?,
                    "expires_at": r.try_get::<DateTime<Utc>>("", "expires_at").map_err(Error::Database)?,
                    "message_sent": r.try_get::<bool>("", "message_sent").map_err(Error::Database)?,
                }))
            })
            .collect()
    }
}
