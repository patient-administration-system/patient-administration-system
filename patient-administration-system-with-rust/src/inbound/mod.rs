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

use crate::db::row::RowExt;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::env;
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
    /// When set, a "different time" reply is answered with real alternative times (CAP-5) instead of being
    /// left for a person.
    rebooking: Option<Arc<crate::rebooking::RebookService>>,
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

/// Who is texting, once the number matches exactly one person.
struct Who {
    person: Uuid,
    lang: Lang,
    tz: chrono_tz::Tz,
}

/// A confirm or cancel text, resolved to one appointment, ready to apply.
#[derive(Clone, Copy)]
struct Reply<'a> {
    message_id: &'a str,
    intent: Intent,
    action: Intent,
    appointment_id: Uuid,
    now: DateTime<Utc>,
}

/// What `target_of` found: an answer to send now, or the action and appointment to apply.
enum Target {
    Answer(Decision),
    Act(Intent, Option<Uuid>),
}

/// A decision that matched no appointment.
fn blank(
    intent: Intent,
    outcome: &'static str,
    person_id: Option<Uuid>,
    reply_text: Option<String>,
    lang: Option<Lang>,
) -> Decision {
    Decision {
        intent,
        outcome,
        person_id,
        appointment_id: None,
        candidates: None,
        pending_intent: None,
        reply_text,
        lang,
    }
}

/// A decision about one appointment, with its reply.
fn done(
    intent: Intent,
    who: &Who,
    outcome: &'static str,
    reply: String,
    appointment_id: Uuid,
) -> Decision {
    Decision {
        intent,
        outcome,
        person_id: Some(who.person),
        appointment_id: Some(appointment_id),
        candidates: None,
        pending_intent: None,
        reply_text: Some(reply),
        lang: Some(who.lang),
    }
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
            rebooking: None,
            default_country_code: env::text("PAS_DEFAULT_COUNTRY_CODE"),
            help_line: env::text("PAS_HELP_LINE"),
        }
    }

    pub fn with_rebooking(mut self, rebooking: Arc<crate::rebooking::RebookService>) -> Self {
        self.rebooking = Some(rebooking);
        self
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
            .execute_raw(Statement::from_sql_and_values(
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
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT from_e164, intent, outcome, reply_text, person_id, appointment_id FROM inbound_texts WHERE message_id = $1",
                [message_id.into()],
            ))
            .await
            .map_err(db_err)?;
        let Some(r) = row else { return Ok(None) };
        let was_from: String = r.col("from_e164")?;
        if was_from != from {
            return Err(Error::conflict(
                "message_id was already used for a message from a different number",
            ));
        }
        Ok(Some(InboundOutcome {
            message_id: message_id.into(),
            outcome: r.col("outcome")?,
            intent: r.col("intent")?,
            reply_text: r.col("reply_text")?,
            language: None,
            person_id: r.col("person_id")?,
            appointment_id: r.col("appointment_id")?,
            duplicate: true,
        }))
    }

    async fn persons_with_number(&self, from: &str) -> Result<Vec<Uuid>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT person_id FROM patient_communication_profiles WHERE mobile_e164 = $1 LIMIT 3",
                [from.into()],
            ))
            .await
            .map_err(db_err)?;
        rows.iter().map(|r| r.col("person_id")).collect()
    }

    /// Appointments to come that a text-channel reminder was sent for recently, soonest first.
    async fn open_reminded(&self, person: Uuid) -> Result<Vec<(Uuid, DateTime<Utc>)>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
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
            .map(|r| Ok((r.col("id")?, r.col("start_datetime")?)))
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
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT outcome, candidates, pending_intent, received_at FROM inbound_texts WHERE from_e164 = $1 \
                 ORDER BY received_at DESC, created_at DESC LIMIT 1",
                [from.into()],
            ))
            .await
            .map_err(db_err)?;
        let Some(r) = row else { return Ok(None) };
        let outcome: String = r.col("outcome")?;
        let at: DateTime<Utc> = r.col("received_at")?;
        if outcome != "asked" || now - at > Duration::minutes(QUESTION_TTL_MINUTES) {
            return Ok(None);
        }
        let cands: Option<serde_json::Value> = r.col("candidates")?;
        let pending: Option<String> = r.col("pending_intent")?;
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

        if let Some(d) = self.keyword_decision(intent, from, single, lang).await? {
            return Ok(d);
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
        let who = Who {
            person,
            lang: lang.unwrap_or(Lang::En),
            tz: messages::timezone(),
        };

        // Work out what is being asked of which appointment.
        let (action, target) = match self.target_of(intent, from, now, &who, ctx).await? {
            Target::Answer(d) => return Ok(d),
            Target::Act(action, target) => (action, target),
        };
        let help = self.help_line.as_deref();
        let (Some(appointment_id), true) = (target, action != Intent::Unknown) else {
            return Ok(blank(
                intent,
                "unknown",
                Some(who.person),
                Some(messages::text(who.lang, Kind::Unknown, "", help)),
                Some(who.lang),
            ));
        };
        if action == Intent::Reschedule {
            return self.reschedule(intent, &who, appointment_id).await;
        }
        let reply = Reply {
            message_id,
            intent,
            action,
            appointment_id,
            now,
        };
        self.apply_reply(&reply, &who, ctx).await
    }

    /// START, STOP and HELP: answered whoever sends them. STOP is honoured against the number and (when we
    /// know who) the person, on every text-style channel whichever one the word arrived on; START lifts it.
    async fn keyword_decision(
        &self,
        intent: Intent,
        from: &str,
        single: Option<Uuid>,
        lang: Option<Lang>,
    ) -> Result<Option<Decision>> {
        use crate::communication::consent as c;
        let help = self.help_line.as_deref();
        let (kind, outcome) = match intent {
            Intent::Start => {
                for ch in c::TEXT_CHANNELS {
                    c::resume(&self.db, &c::address_subject(from), ch).await?;
                    if let Some(p) = single {
                        c::resume(&self.db, &c::person_subject(p), ch).await?;
                    }
                }
                (Kind::Started, "start")
            }
            Intent::Stop => {
                for ch in c::TEXT_CHANNELS {
                    c::record_optout(&self.db, &c::address_subject(from), ch, "reply_stop", None)
                        .await?;
                    if let Some(p) = single {
                        c::record_optout(&self.db, &c::person_subject(p), ch, "reply_stop", None)
                            .await?;
                    }
                }
                (Kind::Stop, "stop")
            }
            Intent::Help => (Kind::Help, "help"),
            _ => return Ok(None),
        };
        let reply = match lang {
            Some(l) => messages::text(l, kind, "", help),
            None => messages::bilingual(kind),
        };
        Ok(Some(blank(intent, outcome, single, Some(reply), lang)))
    }

    /// What a confirm, cancel, reschedule or numbered choice is about: either an answer to send now (a
    /// question back, or nothing open), or the action and the appointment it applies to.
    async fn target_of(
        &self,
        intent: Intent,
        from: &str,
        now: DateTime<Utc>,
        who: &Who,
        ctx: &UserContext,
    ) -> Result<Target> {
        let help = self.help_line.as_deref();
        Ok(match intent {
            Intent::Choice(n) => match self.open_question(from, now).await? {
                Some((ids, pending)) if pending == "rebook" => Target::Answer(
                    self.take_rebook_choice(
                        who.person, &ids, n as usize, intent, who.lang, who.tz, help, ctx,
                    )
                    .await?,
                ),
                Some((ids, pending)) if (1..=ids.len()).contains(&(n as usize)) => {
                    let a = match pending.as_str() {
                        "confirm" => Intent::Confirm,
                        "cancel" => Intent::Cancel,
                        _ => Intent::Reschedule,
                    };
                    Target::Act(a, Some(ids[n as usize - 1]))
                }
                _ => Target::Act(Intent::Unknown, None),
            },
            Intent::Confirm | Intent::Cancel | Intent::Reschedule => {
                let open = self.open_reminded(who.person).await?;
                match open.len() {
                    0 => Target::Answer(blank(
                        intent,
                        "no_open_reminder",
                        Some(who.person),
                        Some(messages::text(who.lang, Kind::NoOpenReminder, "", help)),
                        Some(who.lang),
                    )),
                    1 => Target::Act(intent, Some(open[0].0)),
                    _ => {
                        let times: Vec<String> = open
                            .iter()
                            .map(|(_, at)| messages::when(*at, who.lang, who.tz))
                            .collect();
                        let pending = match intent {
                            Intent::Confirm => "confirm",
                            Intent::Cancel => "cancel",
                            _ => "reschedule",
                        };
                        Target::Answer(Decision {
                            intent,
                            outcome: "asked",
                            person_id: Some(who.person),
                            appointment_id: None,
                            candidates: Some(open.iter().map(|(id, _)| *id).collect()),
                            pending_intent: Some(pending),
                            reply_text: Some(messages::ask(who.lang, &times)),
                            lang: Some(who.lang),
                        })
                    }
                }
            }
            _ => Target::Act(Intent::Unknown, None),
        })
    }

    /// A reschedule: offer real alternatives when they exist (nothing moves until the patient chooses one),
    /// otherwise a person finds the new time.
    async fn reschedule(
        &self,
        intent: Intent,
        who: &Who,
        appointment_id: Uuid,
    ) -> Result<Decision> {
        let help = self.help_line.as_deref();
        if let Some(rebook) = &self.rebooking
            && let Ok(offer) = rebook.offer(appointment_id).await
            && !offer.options.is_empty()
        {
            let whens: Vec<String> = offer
                .options
                .iter()
                .map(|o| messages::when(o.starts_at, who.lang, who.tz))
                .collect();
            let mut ids = vec![appointment_id];
            ids.extend(offer.options.iter().map(|o| o.slot_id));
            return Ok(Decision {
                intent,
                outcome: "asked",
                person_id: Some(who.person),
                appointment_id: Some(appointment_id),
                candidates: Some(ids),
                pending_intent: Some("rebook"),
                reply_text: Some(messages::rebook_options(who.lang, &whens)),
                lang: Some(who.lang),
            });
        }
        let when = match AppointmentRepository::find_by_id(&self.db, appointment_id).await? {
            Some(a) => messages::when(a.start_datetime, who.lang, who.tz),
            None => String::new(),
        };
        Ok(done(
            intent,
            who,
            "needs_staff",
            messages::text(who.lang, Kind::NeedsStaff, &when, help),
            appointment_id,
        ))
    }

    /// A confirm or a cancel, applied through the write-back path so a reply by text, link or API is one rule.
    async fn apply_reply(
        &self,
        reply: &Reply<'_>,
        who: &Who,
        ctx: &UserContext,
    ) -> Result<Decision> {
        let Reply {
            message_id,
            intent,
            action,
            appointment_id,
            now,
        } = *reply;
        let help = self.help_line.as_deref();
        let req = ReplyRequest {
            message_id: message_id.to_string(),
            action: if action == Intent::Confirm {
                ReplyAction::Confirm
            } else {
                ReplyAction::Cancel
            },
            appointment_id,
            person_id: who.person,
            channel: Some("sms".into()),
            barrier_code: None,
            new_slot_id: None,
            modality: None,
            received_at: Some(now),
        };
        let result = self.writeback.apply(req, ctx).await?;
        let when = result
            .appointment
            .as_ref()
            .map(|a| messages::when(a.start_datetime, who.lang, who.tz))
            .unwrap_or_default();
        let (outcome, text) =
            if result.reject_reason.as_deref() == Some(crate::writeback::HELD_FOR_STAFF) {
                (
                    "needs_staff",
                    messages::text(who.lang, Kind::HeldForCall, &when, help),
                )
            } else if result.status == ReplyStatus::Rejected {
                ("rejected", messages::text(who.lang, Kind::Failed, "", help))
            } else {
                let kind = if action == Intent::Confirm {
                    Kind::Confirmed
                } else {
                    Kind::Cancelled
                };
                ("applied", messages::text(who.lang, kind, &when, help))
            };
        Ok(done(intent, who, outcome, text, appointment_id))
    }

    /// The patient chose one of the times a "different time" reply offered. `ids` is the appointment followed
    /// by the offered slots. A slot taken meanwhile, or an offer that has lapsed, is said plainly and the
    /// appointment stays.
    #[allow(
        clippy::too_many_arguments,
        reason = "the recorded text's columns; a struct would only rename them"
    )]
    async fn take_rebook_choice(
        &self,
        person: Uuid,
        ids: &[Uuid],
        n: usize,
        intent: Intent,
        lang: Lang,
        tz: chrono_tz::Tz,
        help: Option<&str>,
        ctx: &UserContext,
    ) -> Result<Decision> {
        let appointment_id = ids.first().copied();
        let mk = |outcome, reply: String| Decision {
            intent,
            outcome,
            person_id: Some(person),
            appointment_id,
            candidates: None,
            pending_intent: None,
            reply_text: Some(reply),
            lang: Some(lang),
        };
        let (Some(appt), Some(rebook), Some(slot)) = (
            appointment_id,
            &self.rebooking,
            ids.get(n).filter(|_| n >= 1),
        ) else {
            return Ok(mk("unknown", messages::text(lang, Kind::Unknown, "", help)));
        };
        let chosen = async {
            let offer_id = rebook
                .standing_offer(appt)
                .await?
                .ok_or_else(|| Error::conflict("the offer has lapsed"))?;
            rebook.choose(appt, offer_id, *slot, ctx).await
        }
        .await;
        Ok(match chosen {
            Ok((_, new)) => mk(
                "applied",
                messages::text(
                    lang,
                    Kind::Rebooked,
                    &messages::when(new.start_datetime, lang, tz),
                    help,
                ),
            ),
            Err(_) => mk("rejected", messages::text(lang, Kind::Failed, "", help)),
        })
    }

    /// Replies nobody could act on, oldest first, for the booking team.
    pub async fn staff_list(&self, limit: i64) -> Result<Vec<StaffItem>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT id, received_at, outcome, intent, person_id, appointment_id, body FROM inbound_texts \
                 WHERE handled_at IS NULL AND outcome IN ('needs_staff','unmatched','ambiguous_number','no_open_reminder','unknown','rejected') \
                 ORDER BY received_at LIMIT $1",
                [crate::paging::fetch(limit).into()],
            ))
            .await
            .map_err(db_err)?;
        rows.iter()
            .map(|r| {
                Ok(StaffItem {
                    id: r.col("id")?,
                    received_at: r.col("received_at")?,
                    outcome: r.col("outcome")?,
                    intent: r.col("intent")?,
                    person_id: r.col("person_id")?,
                    appointment_id: r.col("appointment_id")?,
                    body: r.col("body")?,
                })
            })
            .collect()
    }

    /// A person dealt with it: it leaves the list. Returns false when there was nothing to mark.
    pub async fn mark_handled(&self, id: Uuid) -> Result<bool> {
        let r = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE inbound_texts SET handled_at = now() WHERE id = $1 AND handled_at IS NULL",
                [id.into()],
            ))
            .await
            .map_err(db_err)?;
        Ok(r.rows_affected() > 0)
    }
}
