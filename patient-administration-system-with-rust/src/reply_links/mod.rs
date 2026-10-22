//! Single-use reply links for messages (CAP-13).
//!
//! A message carries one link, `{PAS_LINK_BASE_URL}/r/{token}`. The patient opens it, sees their
//! appointment time, and taps Confirm, Cancel, Reschedule or Help. The design is defensive:
//!
//! - **The token is opaque.** 244 bits of randomness, nothing about the patient or appointment in it. Only
//!   its SHA-256 is stored, so a leaked database gives out no working links.
//! - **Opening a link changes nothing.** `info` is read-only. Email security scanners, link previews and
//!   chat apps follow links; if opening confirmed or cancelled, they would cancel appointments. Only a
//!   deliberate tap, which is a `POST`, acts.
//! - **Cancel asks "Are you sure?"** The first cancel tap returns `needs_confirmation` and changes nothing;
//!   only a second, confirmed one cancels.
//! - **Single use, and it expires** when the appointment starts. Consuming is one atomic `UPDATE`, so two
//!   taps at once act once.
//! - **No clinical detail**: the page shows the time and the language, nothing more, until the patient signs
//!   in to the portal.
//! - The reply is applied through the same write-back as a text reply (CAP-2), so rules, audit and
//!   idempotency are shared.

use crate::db::row::RowExt;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::Serialize;
use uuid::Uuid;

use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::audit::UserContext;
use crate::inbound::messages::{self, Kind, Lang};
use crate::rebooking::RebookService;
use crate::writeback::{ReplyAction, ReplyRequest, ReplyStatus, WriteBackService};
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkAction {
    Confirm,
    Cancel,
    Reschedule,
    Help,
}

impl LinkAction {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "confirm" => Some(Self::Confirm),
            "cancel" => Some(Self::Cancel),
            "reschedule" => Some(Self::Reschedule),
            "help" => Some(Self::Help),
            _ => None,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Confirm => "confirm",
            Self::Cancel => "cancel",
            Self::Reschedule => "reschedule",
            Self::Help => "help",
        }
    }
}

/// What the link page may show: the time and the language, never a reason or a service.
#[derive(Debug, Clone, Serialize)]
pub struct LinkInfo {
    /// `open`, `used` or `expired`.
    pub state: &'static str,
    pub language: &'static str,
    /// The time in the patient's registered language, for a plain-text surface.
    pub when: String,
    /// The start instant and the clinic's time zone, so a page can format it in whatever language it is
    /// shown in (the registered language is only English or Welsh).
    pub starts_at: Option<DateTime<Utc>>,
    pub time_zone: String,
    pub actions: Vec<&'static str>,
}

/// One alternative time offered after Reschedule (CAP-5).
#[derive(Debug, Clone, Serialize)]
pub struct LinkOption {
    pub slot_id: Uuid,
    pub starts_at: DateTime<Utc>,
    /// The time in the patient's registered language, for a plain-text surface.
    pub when: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkOutcome {
    /// `confirmed`, `cancelled`, `rebook_options`, `rebooked`, `held_for_call`, `reschedule_requested`, `needs_confirmation`,
    /// `help` or `rejected`.
    pub status: &'static str,
    pub language: &'static str,
    pub message: String,
    /// For `rebook_options`: the times on offer. Choose one with `POST /api/reply-links/{token}/rebook`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<LinkOption>,
    /// For a cancel that came with a reason (CAS-2): help that fits it, saying cancelling will not affect care.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signpost: Option<String>,
    /// For `rebooked`: when the appointment now starts, so a page can show it in its own language.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub starts_at: Option<DateTime<Utc>>,
    pub time_zone: String,
}

pub struct ReplyLinks {
    db: DatabaseConnection,
    writeback: Arc<WriteBackService>,
    rebook: Arc<RebookService>,
    help_line: Option<String>,
}

use crate::config::env;
use crate::error::LogErr;
pub use crate::link_token::hash;
use crate::link_token::new_token;

/// Mint a link token for an appointment. The plaintext is returned once, for the message; only its hash
/// is stored.
pub async fn mint<C: ConnectionTrait>(
    conn: &C,
    appointment_id: Uuid,
    person_id: Uuid,
    expires_at: DateTime<Utc>,
) -> Result<String> {
    let token = new_token();
    conn.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "INSERT INTO reply_tokens (token_hash, appointment_id, person_id, expires_at) VALUES ($1,$2,$3,$4)",
        [hash(&token).into(), appointment_id.into(), person_id.into(), expires_at.into()],
    ))
    .await
    .map_err(Error::Database)?;
    Ok(token)
}

/// How an outcome is shown to this patient.
struct View {
    lang: Lang,
    tz: chrono_tz::Tz,
}

impl View {
    fn out(&self, status: &'static str, message: String) -> LinkOutcome {
        LinkOutcome {
            status,
            language: self.lang.code(),
            message,
            options: vec![],
            signpost: None,
            starts_at: None,
            time_zone: self.tz.name().to_string(),
        }
    }
}

struct Row {
    appointment_id: Uuid,
    person_id: Uuid,
    expires_at: DateTime<Utc>,
    consumed: bool,
}

impl ReplyLinks {
    pub fn new(
        db: DatabaseConnection,
        writeback: Arc<WriteBackService>,
        rebook: Arc<RebookService>,
    ) -> Self {
        Self {
            db,
            writeback,
            rebook,
            help_line: env::text("PAS_HELP_LINE"),
        }
    }

    async fn row(&self, token: &str) -> Result<Row> {
        // A malformed token is the same "not valid" as an unknown one: no hint which.
        let invalid = || Error::not_found("this link is not valid");
        if token.len() != 64 || !token.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let r = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT appointment_id, person_id, expires_at, consumed_at IS NOT NULL AS consumed FROM reply_tokens WHERE token_hash = $1",
                [hash(token).into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(invalid)?;
        Ok(Row {
            appointment_id: r.col("appointment_id")?,
            person_id: r.col("person_id")?,
            expires_at: r.col("expires_at")?,
            consumed: r.col("consumed")?,
        })
    }

    async fn lang(&self, person: Uuid) -> Result<Lang> {
        Ok(Lang::from_preference(
            crate::contact::preferred_language(&self.db, person)
                .await?
                .as_deref(),
        ))
    }

    /// Read-only. Never changes anything, however many times or by whom it is called.
    pub async fn info(&self, token: &str) -> Result<LinkInfo> {
        let row = self.row(token).await?;
        let lang = self.lang(row.person_id).await?;
        let appt = AppointmentRepository::find_by_id(&self.db, row.appointment_id).await?;
        let starts_at = appt.as_ref().map(|a| a.start_datetime);
        let when = appt
            .map(|a| messages::when(a.start_datetime, lang, messages::timezone()))
            .unwrap_or_default();
        let state = if row.consumed {
            "used"
        } else if row.expires_at <= Utc::now() {
            "expired"
        } else {
            "open"
        };
        let actions = if state == "open" {
            vec!["confirm", "cancel", "reschedule", "help"]
        } else {
            vec![]
        };
        Ok(LinkInfo {
            state,
            language: lang.code(),
            when,
            starts_at,
            time_zone: messages::timezone().name().to_string(),
            actions,
        })
    }

    /// A deliberate tap. `confirmed` is the second step of Cancel and is ignored for the rest.
    pub async fn act(
        &self,
        token: &str,
        action: LinkAction,
        confirmed: bool,
        ctx: &UserContext,
    ) -> Result<LinkOutcome> {
        self.act_with(token, action, confirmed, None, ctx).await
    }

    /// [`Self::act`] where a cancelling patient may say why (CAS-2). The reason is recorded with the cancel
    /// and answered with help that fits it. It is optional and never required to cancel.
    pub async fn act_with(
        &self,
        token: &str,
        action: LinkAction,
        confirmed: bool,
        barrier: Option<crate::models::appointment::BarrierCode>,
        ctx: &UserContext,
    ) -> Result<LinkOutcome> {
        let row = self.row(token).await?;
        let view = View {
            lang: self.lang(row.person_id).await?,
            tz: messages::timezone(),
        };
        let help = self.help_line.as_deref();

        if row.consumed {
            return Err(Error::conflict("this link has already been used"));
        }
        if row.expires_at <= Utc::now() {
            return Err(Error::validation("this link has expired"));
        }
        match action {
            // Help is information: it does not use the link up.
            LinkAction::Help => {
                return Ok(view.out("help", messages::text(view.lang, Kind::Help, "", help)));
            }
            // The first Cancel tap only asks; nothing changes.
            LinkAction::Cancel if !confirmed => {
                return Ok(view.out("needs_confirmation", messages::are_you_sure(view.lang)));
            }
            _ => {}
        }

        // Reschedule first offers real alternative times (CAP-5). Offering is not acting: the link is not
        // used up and the appointment stands, so the patient can look, go back, or choose. If there is
        // nothing to offer, a person takes over (below).
        if action == LinkAction::Reschedule
            && let Some(offered) = self.alternatives(&row, &view).await
        {
            return Ok(offered);
        }

        // Claim the link atomically; whoever fails to claim it was too late.
        let hash = hash(token);
        self.claim(&hash, action).await?;

        let appt = AppointmentRepository::find_by_id(&self.db, row.appointment_id).await?;
        let when = appt
            .map(|a| messages::when(a.start_datetime, view.lang, view.tz))
            .unwrap_or_default();
        let result = if action == LinkAction::Reschedule {
            self.request_reschedule(&row, &view, &when).await
        } else {
            self.apply_through_writeback(&row, &view, action, barrier, &hash, &when, ctx)
                .await
        };
        if result.is_err() {
            self.release(&hash).await;
        }
        result
    }

    /// Real alternative times for a reschedule, if there are any to offer.
    async fn alternatives(&self, row: &Row, view: &View) -> Option<LinkOutcome> {
        let offer = self.rebook.offer(row.appointment_id).await.ok()?;
        if offer.options.is_empty() {
            return None;
        }
        let mut o = view.out("rebook_options", String::new());
        o.options = offer
            .options
            .iter()
            .map(|s| LinkOption {
                slot_id: s.slot_id,
                starts_at: s.starts_at,
                when: messages::when(s.starts_at, view.lang, view.tz),
            })
            .collect();
        Some(o)
    }

    /// Use the link up, atomically. Whoever fails to claim it was too late.
    async fn claim(&self, hash: &str, action: LinkAction) -> Result<()> {
        let claimed = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE reply_tokens SET consumed_at = now(), consumed_action = $2 \
                 WHERE token_hash = $1 AND consumed_at IS NULL AND expires_at > now()",
                [hash.to_string().into(), action.as_str().into()],
            ))
            .await
            .map_err(Error::Database)?;
        if claimed.rows_affected() == 0 {
            return Err(Error::conflict("this link has already been used"));
        }
        Ok(())
    }

    /// Give the link back after a refused or failed action, so the patient can try again.
    async fn release(&self, hash: &str) {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE reply_tokens SET consumed_at = NULL, consumed_action = NULL WHERE token_hash = $1",
                [hash.to_string().into()],
            ))
            .await
            .log_error("giving a reply link back");
    }

    /// Leads to rebook options (CAP-5); until then a person finds the new time. The appointment stands.
    async fn request_reschedule(&self, row: &Row, view: &View, when: &str) -> Result<LinkOutcome> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) VALUES ($1,$2,$3,'reschedule_request', now()) \
                 ON CONFLICT (appointment_id, reason) DO NOTHING",
                [Uuid::new_v4().into(), row.appointment_id.into(), row.person_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        Ok(view.out(
            "reschedule_requested",
            messages::text(view.lang, Kind::NeedsStaff, when, self.help_line.as_deref()),
        ))
    }

    /// A confirm or a cancel, applied through the write-back path so a reply by link, text or API is one rule.
    #[allow(
        clippy::too_many_arguments,
        reason = "the parts of one link action as the caller holds them"
    )]
    async fn apply_through_writeback(
        &self,
        row: &Row,
        view: &View,
        action: LinkAction,
        barrier: Option<crate::models::appointment::BarrierCode>,
        hash: &str,
        when: &str,
        ctx: &UserContext,
    ) -> Result<LinkOutcome> {
        let help = self.help_line.as_deref();
        let req = ReplyRequest {
            message_id: format!("link-{hash}"),
            action: if action == LinkAction::Confirm {
                ReplyAction::Confirm
            } else {
                ReplyAction::Cancel
            },
            appointment_id: row.appointment_id,
            person_id: row.person_id,
            channel: Some("link".into()),
            barrier_code: if action == LinkAction::Cancel {
                barrier
            } else {
                None
            },
            new_slot_id: None,
            modality: None,
            received_at: None,
        };
        let r = self.writeback.apply(req, ctx).await?;
        Ok(if r.status == ReplyStatus::Applied {
            let (kind, status) = if action == LinkAction::Confirm {
                (Kind::Confirmed, "confirmed")
            } else {
                (Kind::Cancelled, "cancelled")
            };
            let mut o = view.out(status, messages::text(view.lang, kind, when, help));
            if action == LinkAction::Cancel {
                o.signpost = barrier.and_then(|b| messages::signpost(view.lang, b));
            }
            o
        } else if r.reject_reason.as_deref() == Some(crate::writeback::HELD_FOR_STAFF) {
            view.out(
                "held_for_call",
                messages::text(view.lang, Kind::HeldForCall, when, help),
            )
        } else {
            view.out(
                "rejected",
                messages::text(view.lang, Kind::Failed, "", help),
            )
        })
    }

    /// The patient chose one of the offered times. The link is used up by this tap, atomically, and given
    /// back if the move is refused (for example the slot was just taken), so the patient can pick again.
    pub async fn choose(
        &self,
        token: &str,
        slot_id: Uuid,
        ctx: &UserContext,
    ) -> Result<LinkOutcome> {
        let row = self.row(token).await?;
        let lang = self.lang(row.person_id).await?;
        let tz = messages::timezone();
        if row.consumed {
            return Err(Error::conflict("this link has already been used"));
        }
        if row.expires_at <= Utc::now() {
            return Err(Error::validation("this link has expired"));
        }
        let offer_id = self
            .rebook
            .standing_offer(row.appointment_id)
            .await?
            .ok_or_else(|| {
                Error::conflict("there are no times on offer; ask for other times again")
            })?;

        let hash = hash(token);
        let claimed = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE reply_tokens SET consumed_at = now(), consumed_action = 'rebook' \
                 WHERE token_hash = $1 AND consumed_at IS NULL AND expires_at > now()",
                [hash.clone().into()],
            ))
            .await
            .map_err(Error::Database)?;
        if claimed.rows_affected() == 0 {
            return Err(Error::conflict("this link has already been used"));
        }
        match self
            .rebook
            .choose(row.appointment_id, offer_id, slot_id, ctx)
            .await
        {
            Ok((_, new)) => Ok(LinkOutcome {
                status: "rebooked",
                language: lang.code(),
                message: messages::text(
                    lang,
                    Kind::Rebooked,
                    &messages::when(new.start_datetime, lang, tz),
                    None,
                ),
                options: vec![],
                signpost: None,
                starts_at: Some(new.start_datetime),
                time_zone: tz.name().to_string(),
            }),
            Err(e) => {
                self
                    .db
                    .execute_raw(Statement::from_sql_and_values(
                        DbBackend::Postgres,
                        "UPDATE reply_tokens SET consumed_at = NULL, consumed_action = NULL WHERE token_hash = $1",
                        [hash.into()],
                    ))
                    .await
.log_error("giving a reply link back");
                Err(e)
            }
        }
    }
}
