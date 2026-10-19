//! Mapping a patient's raw text reply to an appointment (PILOT-4).
//!
//! A carrier posts what arrived: the sender's number, the words, the carrier's message id. This module
//! decides what the patient meant ([`parser`]), which appointment they meant, applies the change through
//! the write-back service, and returns the text to send back ([`messages`], English and Welsh).
//!
//! **Which appointment.** The number is matched to the one patient whose profile holds it. Their
//! candidates are the appointments still to come that we sent a text-channel reminder for in the last
//! 14 days: a reply answers a reminder. Exactly one candidate: act on it. Several: ask "which?" and
//! remember the question for 30 minutes, so the next message (a number) completes it. None: say so.
//!
//! **What it will not do.**
//! - It never acts on a message it half understands (see the parser).
//! - It never acts for a number shared by two patients, or one it cannot match: those, and every
//!   reschedule request, go on a staff list. Replying would tell a stranger something about a patient.
//! - A reschedule reply never cancels anything: the appointment stands until a person moves it.
//! - STOP is always acted on, even from an unknown or shared number: it opts the number (and the person,
//!   if known) out of that channel (`communication::consent`), and START lifts it.
//!
//! Every inbound text is kept in `inbound_texts`, which is also the idempotency key for the carrier's id.

pub mod messages;
pub mod parser;

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::contact::normalise_e164;
use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::audit::UserContext;
use crate::writeback::{ReplyAction, ReplyRequest, ReplyStatus, WriteBackService};
use crate::{Error, Result};
use messages::{Kind, Lang};
use parser::Intent;

/// How long an "which appointment?" question waits for its answer.
pub const QUESTION_TTL_MINUTES: i64 = 30;
/// A reply answers a reminder sent within this many days.
pub const REMINDER_WINDOW_DAYS: i64 = 14;
const MAX_BODY: usize = 500;
const MAX_CANDIDATES: usize = 9;

#[derive(Debug, Clone, Deserialize)]
pub struct InboundText {
    /// The carrier's id for this message: the idempotency key.
    pub message_id: String,
    pub from: String,
    pub body: String,
    #[serde(default)]
    pub carrier: Option<String>,
    /// The channel it arrived on: `sms` (the default), `whatsapp` or `apple_messages`. A STOP stops
    /// that channel.
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub received_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InboundOutcome {
    pub message_id: String,
    /// `applied`, `rejected`, `asked`, `needs_staff`, `stop`, `help`, `unknown`, `unmatched`,
    /// `ambiguous_number` or `no_open_reminder`.
    pub outcome: String,
    pub intent: String,
    /// What to text back, or none (never to an unmatched or shared number).
    pub reply_text: Option<String>,
    pub language: Option<String>,
    pub person_id: Option<Uuid>,
    pub appointment_id: Option<Uuid>,
    pub duplicate: bool,
}

/// A row for the staff list.
#[derive(Debug, Clone, Serialize)]
pub struct StaffItem {
    pub id: Uuid,
    pub received_at: DateTime<Utc>,
    pub outcome: String,
    pub intent: String,
    pub person_id: Option<Uuid>,
    pub appointment_id: Option<Uuid>,
    /// The patient's words. Staff need them to act; they are personal data (see the inventory).
    pub body: String,
}

pub struct InboundService {
    db: DatabaseConnection,
    writeback: Arc<WriteBackService>,
    default_country_code: Option<String>,
    help_line: Option<String>,
}

struct Decision {
    intent: Intent,
    outcome: &'static str,
    person_id: Option<Uuid>,
    appointment_id: Option<Uuid>,
    candidates: Option<Vec<Uuid>>,
    pending_intent: Option<&'static str>,
    reply_text: Option<String>,
    lang: Option<Lang>,
}

fn db_err(e: sea_orm::DbErr) -> Error {
    Error::Database(e)
}

fn intent_str(i: Intent) -> &'static str {
    match i {
        Intent::Confirm => "confirm",
        Intent::Cancel => "cancel",
        Intent::Reschedule => "reschedule",
        Intent::Stop => "stop",
        Intent::Start => "start",
        Intent::Help => "help",
        Intent::Choice(_) => "choice",
        Intent::Unknown => "unknown",
    }
}

impl InboundService {
    pub fn new(db: DatabaseConnection, writeback: Arc<WriteBackService>) -> Self {
        Self {
            db,
            writeback,
            default_country_code: std::env::var("PAS_DEFAULT_COUNTRY_CODE")
                .ok()
                .filter(|s| !s.is_empty()),
            help_line: std::env::var("PAS_HELP_LINE")
                .ok()
                .filter(|s| !s.trim().is_empty()),
        }
    }

    pub async fn handle(&self, text: InboundText, ctx: &UserContext) -> Result<InboundOutcome> {
        let message_id = text.message_id.trim().to_string();
        if message_id.is_empty()
            || message_id.len() > 200
            || message_id.chars().any(char::is_control)
        {
            return Err(Error::validation(
                "message_id must be 1 to 200 characters with no control characters",
            ));
        }
        if text.body.chars().count() > 1000 {
            return Err(Error::validation("body is longer than any text message"));
        }
        let from = normalise_e164(&text.from, self.default_country_code.as_deref())?;
        if let Some(stored) = self.stored(&message_id, &from).await? {
            return Ok(stored);
        }
        let now = text
            .received_at
            .map_or_else(Utc::now, |t| t.min(Utc::now()));
        let intent = parser::parse(&text.body);
        let channel = match text
            .channel
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
        {
            None => "sms",
            Some(c @ ("sms" | "whatsapp" | "apple_messages")) => c,
            Some(_) => {
                return Err(Error::validation(
                    "channel must be sms, whatsapp or apple_messages",
                ));
            }
        };
        let d = self
            .decide(&message_id, &from, intent, channel, now, ctx)
            .await?;

        let body: String = text.body.chars().take(MAX_BODY).collect();
        let inserted = self
            .db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO inbound_texts (id, message_id, carrier, from_e164, body, received_at, person_id, intent, outcome, \
                   appointment_id, candidates, pending_intent, reply_text) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) ON CONFLICT (message_id) DO NOTHING",
                [
                    Uuid::new_v4().into(),
                    message_id.clone().into(),
                    text.carrier.clone().into(),
                    from.clone().into(),
                    body.into(),
                    now.into(),
                    d.person_id.into(),
                    intent_str(d.intent).into(),
                    d.outcome.into(),
                    d.appointment_id.into(),
                    d.candidates.as_ref().map(|c| serde_json::json!(c)).into(),
                    d.pending_intent.map(String::from).into(),
                    d.reply_text.clone().into(),
                ],
            ))
            .await
            .map_err(db_err)?;
        if inserted.rows_affected() == 0 {
            // Two deliveries of the same message raced; the other one's outcome stands.
            return self
                .stored(&message_id, &from)
                .await?
                .ok_or_else(|| Error::internal("inbound text vanished"));
        }
        Ok(InboundOutcome {
            message_id,
            outcome: d.outcome.into(),
            intent: intent_str(d.intent).into(),
            reply_text: d.reply_text,
            language: d.lang.map(|l| l.code().into()),
            person_id: d.person_id,
            appointment_id: d.appointment_id,
            duplicate: false,
        })
    }

    async fn stored(&self, message_id: &str, from: &str) -> Result<Option<InboundOutcome>> {
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT from_e164, intent, outcome, reply_text, person_id, appointment_id FROM inbound_texts WHERE message_id = $1",
                [message_id.into()],
            ))
            .await
            .map_err(db_err)?;
        let Some(r) = row else { return Ok(None) };
        let was_from: String = r.try_get("", "from_e164").map_err(db_err)?;
        if was_from != from {
            return Err(Error::conflict(
                "message_id was already used for a message from a different number",
            ));
        }
        Ok(Some(InboundOutcome {
            message_id: message_id.into(),
            outcome: r.try_get("", "outcome").map_err(db_err)?,
            intent: r.try_get("", "intent").map_err(db_err)?,
            reply_text: r.try_get("", "reply_text").map_err(db_err)?,
            language: None,
            person_id: r.try_get("", "person_id").map_err(db_err)?,
            appointment_id: r.try_get("", "appointment_id").map_err(db_err)?,
            duplicate: true,
        }))
    }

    async fn persons_with_number(&self, from: &str) -> Result<Vec<Uuid>> {
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT person_id FROM patient_communication_profiles WHERE mobile_e164 = $1 LIMIT 3",
                [from.into()],
            ))
            .await
            .map_err(db_err)?;
        rows.iter()
            .map(|r| r.try_get("", "person_id").map_err(db_err))
            .collect()
    }

    /// Appointments to come that a text-channel reminder was sent for recently, soonest first.
    async fn open_reminded(&self, person: Uuid) -> Result<Vec<(Uuid, DateTime<Utc>)>> {
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT a.id, a.start_datetime FROM appointments a \
                 WHERE a.person_id = $1 AND a.status = 'booked' AND a.deleted_at IS NULL AND a.start_datetime > now() \
                   AND EXISTS (SELECT 1 FROM generated_letters l WHERE l.appointment_id = a.id AND l.status = 'sent' \
                               AND l.channel IN ('sms','whatsapp','apple_messages') \
                               AND l.created_at > now() - ($2 || ' days')::interval) \
                 ORDER BY a.start_datetime LIMIT $3",
                [person.into(), REMINDER_WINDOW_DAYS.to_string().into(), (MAX_CANDIDATES as i64).into()],
            ))
            .await
            .map_err(db_err)?;
        rows.iter()
            .map(|r| {
                Ok((
                    r.try_get("", "id").map_err(db_err)?,
                    r.try_get("", "start_datetime").map_err(db_err)?,
                ))
            })
            .collect()
    }

    /// The unanswered question this number was last asked, if it is still fresh.
    async fn open_question(
        &self,
        from: &str,
        now: DateTime<Utc>,
    ) -> Result<Option<(Vec<Uuid>, String)>> {
        let row = self
            .db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT outcome, candidates, pending_intent, received_at FROM inbound_texts WHERE from_e164 = $1 \
                 ORDER BY received_at DESC, created_at DESC LIMIT 1",
                [from.into()],
            ))
            .await
            .map_err(db_err)?;
        let Some(r) = row else { return Ok(None) };
        let outcome: String = r.try_get("", "outcome").map_err(db_err)?;
        let at: DateTime<Utc> = r.try_get("", "received_at").map_err(db_err)?;
        if outcome != "asked" || now - at > Duration::minutes(QUESTION_TTL_MINUTES) {
            return Ok(None);
        }
        let cands: Option<serde_json::Value> = r.try_get("", "candidates").map_err(db_err)?;
        let pending: Option<String> = r.try_get("", "pending_intent").map_err(db_err)?;
        let ids: Vec<Uuid> = cands
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        Ok(pending.map(|p| (ids, p)))
    }

    async fn decide(
        &self,
        message_id: &str,
        from: &str,
        intent: Intent,
        _channel: &str,
        now: DateTime<Utc>,
        ctx: &UserContext,
    ) -> Result<Decision> {
        let tz = messages::timezone();
        let help = self.help_line.as_deref();
        let blank = |intent, outcome, person_id, reply_text, lang| Decision {
            intent,
            outcome,
            person_id,
            appointment_id: None,
            candidates: None,
            pending_intent: None,
            reply_text,
            lang,
        };

        let persons = self.persons_with_number(from).await?;
        let single = if persons.len() == 1 {
            Some(persons[0])
        } else {
            None
        };
        let lang = match single {
            Some(p) => Some(Lang::from_preference(
                crate::contact::preferred_language(&self.db, p)
                    .await?
                    .as_deref(),
            )),
            None => None,
        };

        // START lifts a STOP, for the number and (when we know who) the person.
        if intent == Intent::Start {
            use crate::communication::consent as c;
            for ch in c::TEXT_CHANNELS {
                c::resume(&self.db, &c::address_subject(from), ch).await?;
                if let Some(p) = single {
                    c::resume(&self.db, &c::person_subject(p), ch).await?;
                }
            }
            let reply = match lang {
                Some(l) => messages::text(l, Kind::Started, "", help),
                None => messages::bilingual(Kind::Started),
            };
            return Ok(blank(intent, "start", single, Some(reply), lang));
        }
        // STOP is honoured whoever sends it, against the number and (when we know who) the person.
        if intent == Intent::Stop {
            use crate::communication::consent as c;
            // "Stop texting me" means every text-style channel, whichever one the word arrived on.
            for ch in c::TEXT_CHANNELS {
                c::record_optout(&self.db, &c::address_subject(from), ch, "reply_stop", None)
                    .await?;
                if let Some(p) = single {
                    c::record_optout(&self.db, &c::person_subject(p), ch, "reply_stop", None)
                        .await?;
                }
            }
            let reply = match lang {
                Some(l) => messages::text(l, Kind::Stop, "", help),
                None => messages::bilingual(Kind::Stop),
            };
            return Ok(blank(intent, "stop", single, Some(reply), lang));
        }
        if intent == Intent::Help {
            let reply = match lang {
                Some(l) => messages::text(l, Kind::Help, "", help),
                None => messages::bilingual(Kind::Help),
            };
            return Ok(blank(intent, "help", single, Some(reply), lang));
        }
        let Some(person) = single else {
            // Nobody, or more than one: say nothing, let staff see it.
            let outcome = if persons.is_empty() {
                "unmatched"
            } else {
                "ambiguous_number"
            };
            return Ok(blank(intent, outcome, None, None, None));
        };
        let lang = lang.unwrap_or(Lang::En);

        // Work out what is being asked of which appointment.
        let (action, target): (Intent, Option<Uuid>) = match intent {
            Intent::Choice(n) => match self.open_question(from, now).await? {
                Some((ids, pending)) if (1..=ids.len()).contains(&(n as usize)) => {
                    let a = match pending.as_str() {
                        "confirm" => Intent::Confirm,
                        "cancel" => Intent::Cancel,
                        _ => Intent::Reschedule,
                    };
                    (a, Some(ids[n as usize - 1]))
                }
                _ => (Intent::Unknown, None),
            },
            Intent::Confirm | Intent::Cancel | Intent::Reschedule => {
                let open = self.open_reminded(person).await?;
                match open.len() {
                    0 => {
                        return Ok(blank(
                            intent,
                            "no_open_reminder",
                            Some(person),
                            Some(messages::text(lang, Kind::NoOpenReminder, "", help)),
                            Some(lang),
                        ));
                    }
                    1 => (intent, Some(open[0].0)),
                    _ => {
                        let times: Vec<String> = open
                            .iter()
                            .map(|(_, at)| messages::when(*at, lang, tz))
                            .collect();
                        let pending = match intent {
                            Intent::Confirm => "confirm",
                            Intent::Cancel => "cancel",
                            _ => "reschedule",
                        };
                        return Ok(Decision {
                            intent,
                            outcome: "asked",
                            person_id: Some(person),
                            appointment_id: None,
                            candidates: Some(open.iter().map(|(id, _)| *id).collect()),
                            pending_intent: Some(pending),
                            reply_text: Some(messages::ask(lang, &times)),
                            lang: Some(lang),
                        });
                    }
                }
            }
            _ => (Intent::Unknown, None),
        };

        let (Some(appointment_id), true) = (target, action != Intent::Unknown) else {
            return Ok(blank(
                intent,
                "unknown",
                Some(person),
                Some(messages::text(lang, Kind::Unknown, "", help)),
                Some(lang),
            ));
        };

        let done = |outcome, reply: String, appt| Decision {
            intent,
            outcome,
            person_id: Some(person),
            appointment_id: Some(appt),
            candidates: None,
            pending_intent: None,
            reply_text: Some(reply),
            lang: Some(lang),
        };

        if action == Intent::Reschedule {
            // Never moves or cancels anything: a person finds the new time.
            let when = match AppointmentRepository::find_by_id(&self.db, appointment_id).await? {
                Some(a) => messages::when(a.start_datetime, lang, tz),
                None => String::new(),
            };
            return Ok(done(
                "needs_staff",
                messages::text(lang, Kind::NeedsStaff, &when, help),
                appointment_id,
            ));
        }

        let req = ReplyRequest {
            message_id: message_id.to_string(),
            action: if action == Intent::Confirm {
                ReplyAction::Confirm
            } else {
                ReplyAction::Cancel
            },
            appointment_id,
            person_id: person,
            channel: Some("sms".into()),
            barrier_code: None,
            new_slot_id: None,
            modality: None,
            received_at: Some(now),
        };
        let result = self.writeback.apply(req, ctx).await?;
        if result.status == ReplyStatus::Rejected {
            return Ok(done(
                "rejected",
                messages::text(lang, Kind::Failed, "", help),
                appointment_id,
            ));
        }
        let when = result
            .appointment
            .as_ref()
            .map(|a| messages::when(a.start_datetime, lang, tz))
            .unwrap_or_default();
        let kind = if action == Intent::Confirm {
            Kind::Confirmed
        } else {
            Kind::Cancelled
        };
        Ok(done(
            "applied",
            messages::text(lang, kind, &when, help),
            appointment_id,
        ))
    }

    /// Replies nobody could act on, oldest first, for the booking team.
    pub async fn staff_list(&self, limit: i64) -> Result<Vec<StaffItem>> {
        let rows = self
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id, received_at, outcome, intent, person_id, appointment_id, body FROM inbound_texts \
                 WHERE handled_at IS NULL AND outcome IN ('needs_staff','unmatched','ambiguous_number','no_open_reminder','unknown','rejected') \
                 ORDER BY received_at LIMIT $1",
                [limit.clamp(1, 500).into()],
            ))
            .await
            .map_err(db_err)?;
        rows.iter()
            .map(|r| {
                Ok(StaffItem {
                    id: r.try_get("", "id").map_err(db_err)?,
                    received_at: r.try_get("", "received_at").map_err(db_err)?,
                    outcome: r.try_get("", "outcome").map_err(db_err)?,
                    intent: r.try_get("", "intent").map_err(db_err)?,
                    person_id: r.try_get("", "person_id").map_err(db_err)?,
                    appointment_id: r.try_get("", "appointment_id").map_err(db_err)?,
                    body: r.try_get("", "body").map_err(db_err)?,
                })
            })
            .collect()
    }

    /// A person dealt with it: it leaves the list. Returns false when there was nothing to mark.
    pub async fn mark_handled(&self, id: Uuid) -> Result<bool> {
        let r = self
            .db
            .execute(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE inbound_texts SET handled_at = now() WHERE id = $1 AND handled_at IS NULL",
                [id.into()],
            ))
            .await
            .map_err(db_err)?;
        Ok(r.rows_affected() > 0)
    }
}
