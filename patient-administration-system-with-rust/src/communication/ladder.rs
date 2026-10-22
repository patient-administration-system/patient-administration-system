//! The reminder ladder (CAP-13): every appointment climbs the same steps, and a patient who stays silent
//! is escalated, whatever any score says.
//!
//! ```text
//!  T-7 days  main    "Can't come? Tell us now so someone else can have the slot"   (preferred channel)
//!  T-4 days  second  everyone with no reply action: another channel, SMS first
//!  T-3 days  call    still silent: a person phones (staff call list)
//!  T-24h     final   short nudge to everyone still booked
//! ```
//!
//! Why these times: the PAS counts a cancellation as an *early release* only with at least
//! `EARLY_RELEASE_DAYS` (3) days' notice, because a slot freed later is hard to refill. A "can't come?" at
//! T-7 and a second touch at T-4 both leave a patient time to reply and still give 3 days or more.
//!
//! **Escalation is on a reply, never on an open.** Mail apps pre-fetch images, so an "opened" is not
//! evidence of anything. The only test is whether the patient has taken an action: confirmed, asked to
//! reschedule, or used a link. An open is never recorded, and never counts for or against a patient.
//!
//! **A step runs once** (`reminder_steps` has one row per appointment per step) and is logged with its
//! channel, language and outcome, so results can be split by language and, with CAP-11, deprivation.
//! If a scan finds several steps due at once (an appointment booked late), only the latest runs and the
//! earlier ones are logged as `skipped_late`: nobody gets three reminders in a minute.
//!
//! Guardrails, all checked on every send: a touch cap per appointment, quiet hours (a held step stays due
//! and goes in the morning), STOP and opt-out (`communication::consent`), no clinical detail (the wording),
//! and the language variant of the patient's preference.

use crate::db::row::RowExt;
use chrono::{DateTime, Duration, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use std::sync::Arc;
use uuid::Uuid;

use super::CommunicationService;
use super::reminders::{MAX_FAILED_ATTEMPTS, REMINDER_TEMPLATE_NAME, ReminderService};
use crate::config::env;
use crate::db::repositories::appointment::AppointmentRepository;
use crate::db::repositories::audit::UserContext;
use crate::db::repositories::letter::LetterRepository;
use crate::models::appointment::Appointment;
use crate::models::communication::{DeliveryChannel, LetterStatus};
use crate::{Error, Result};

pub const DEFAULT_STEPS: &str = "main=7d,second=4d,call=3d,final=24h";
pub const DEFAULT_MAX_TOUCHES: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    Main,
    Second,
    Call,
    Final,
}

impl Step {
    pub fn as_str(self) -> &'static str {
        match self {
            Step::Main => "main",
            Step::Second => "second",
            Step::Call => "call",
            Step::Final => "final",
        }
    }
    fn parse(s: &str) -> Option<Step> {
        match s {
            "main" => Some(Step::Main),
            "second" => Some(Step::Second),
            "call" => Some(Step::Call),
            "final" => Some(Step::Final),
            _ => None,
        }
    }
    /// The message sent at this step (the call step sends none: a person phones).
    pub fn template_name(self) -> Option<&'static str> {
        match self {
            Step::Main => Some(REMINDER_TEMPLATE_NAME),
            Step::Second => Some("appointment_reminder_second"),
            Step::Final => Some("appointment_reminder_final"),
            Step::Call => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepSpec {
    pub step: Step,
    /// How long before the appointment this step is due.
    pub before: Duration,
}

/// The built-in ladder, `DEFAULT_STEPS`. The unit tests parse it, so it cannot fail at run time.
#[allow(clippy::expect_used, reason = "a constant that the unit tests parse")]
fn default_steps() -> Vec<StepSpec> {
    parse_steps(DEFAULT_STEPS).expect("DEFAULT_STEPS parses")
}

/// Parse `main=7d,second=4d,call=3d,final=24h`. Units are `d` and `h`; `main` is required; steps run in
/// the order of decreasing lead time, so the list must already be in that order.
pub fn parse_steps(spec: &str) -> Result<Vec<StepSpec>> {
    let bad = |m: String| Error::validation(format!("PAS_REMINDER_STEPS: {m}"));
    let mut out: Vec<StepSpec> = Vec::new();
    for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (name, dur) = item
            .split_once('=')
            .ok_or_else(|| bad(format!("{item:?} is not step=duration")))?;
        let step = Step::parse(name.trim())
            .ok_or_else(|| bad(format!("unknown step {name:?} (main, second, call, final)")))?;
        let dur = dur.trim();
        let (num, unit) = dur.split_at(dur.len().saturating_sub(1));
        let n: i64 = num
            .parse()
            .map_err(|_| bad(format!("{dur:?} is not like 7d or 24h")))?;
        let before = match unit {
            "d" => Duration::days(n),
            "h" => Duration::hours(n),
            _ => return Err(bad(format!("{dur:?} must end in d or h"))),
        };
        if before <= Duration::zero() || before > Duration::days(60) {
            return Err(bad(format!("{dur:?} must be between 1 hour and 60 days")));
        }
        if out.iter().any(|s| s.step == step) {
            return Err(bad(format!("{name} is listed twice")));
        }
        if let Some(prev) = out.last()
            && before >= prev.before
        {
            return Err(bad(
                "steps must be listed from the longest lead time to the shortest".into(),
            ));
        }
        out.push(StepSpec { step, before });
    }
    if !out.iter().any(|s| s.step == Step::Main) {
        return Err(bad("main is required".into()));
    }
    Ok(out)
}

/// Steps whose time has come for an appointment starting at `start`, in time order.
pub fn due_steps(
    steps: &[StepSpec],
    start: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Vec<(Step, DateTime<Utc>)> {
    let mut v: Vec<(Step, DateTime<Utc>)> = steps
        .iter()
        .map(|s| (s.step, start - s.before))
        .filter(|(_, at)| *at <= now)
        .collect();
    v.sort_by_key(|(_, at)| *at);
    v
}

/// The channels to try, in order. The first message uses the patient's preferred channel; the second
/// touch tries *other* channels first, text before the rest, and falls back to the first channel only if
/// nothing else can reach them.
pub fn channel_order(
    offered: &[DeliveryChannel],
    preferred: Option<DeliveryChannel>,
    step: Step,
    first_channel: Option<DeliveryChannel>,
) -> Vec<DeliveryChannel> {
    let base = super::reminders::prefer_first(offered, preferred);
    if step != Step::Second {
        return base;
    }
    let Some(used) = first_channel else {
        return base;
    };
    let mut others: Vec<DeliveryChannel> = base.iter().copied().filter(|c| *c != used).collect();
    // Text first: it is read soonest.
    others.sort_by_key(|c| if *c == DeliveryChannel::Sms { 0 } else { 1 });
    if base.contains(&used) {
        others.push(used);
    }
    others
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct LadderReport {
    pub considered: usize,
    pub sent: usize,
    pub call_tasks: usize,
    pub skipped_replied: usize,
    pub skipped_late: usize,
    pub deferred_quiet_hours: usize,
    pub skipped_opted_out: usize,
    pub skipped_no_channel: usize,
    pub capped: usize,
    pub failed: usize,
}

enum Delivery {
    Sent {
        channel: DeliveryChannel,
        locale: String,
        letter_id: Uuid,
    },
    Deferred,
    OptedOut,
    NoChannel,
    Failed,
}

pub struct LadderService {
    db: DatabaseConnection,
    communication: Arc<CommunicationService>,
    reminders: ReminderService,
    steps: Vec<StepSpec>,
    max_touches: usize,
    link_base: Option<String>,
    quiet_hours: Option<super::consent::QuietHours>,
}

impl LadderService {
    pub fn new(db: DatabaseConnection, communication: Arc<CommunicationService>) -> Self {
        Self {
            reminders: ReminderService::new(db.clone(), communication.clone()),
            db,
            communication,
            steps: default_steps(),
            max_touches: DEFAULT_MAX_TOUCHES,
            link_base: None,
            quiet_hours: None,
        }
    }

    pub fn with_steps(mut self, steps: Vec<StepSpec>) -> Self {
        self.steps = steps;
        self
    }
    pub fn with_max_touches(mut self, n: usize) -> Self {
        self.max_touches = n;
        self
    }
    /// The public address of the page that serves reply links (the UI), without a trailing slash.
    pub fn with_link_base(mut self, base: Option<String>) -> Self {
        self.link_base = base
            .map(|b| b.trim_end_matches('/').to_string())
            .filter(|b| !b.is_empty());
        self
    }
    pub fn with_quiet_hours(mut self, q: Option<super::consent::QuietHours>) -> Self {
        self.quiet_hours = q;
        self
    }

    /// From `PAS_REMINDER_STEPS`, `PAS_REMINDER_MAX_TOUCHES`, `PAS_LINK_BASE_URL`, `PAS_QUIET_HOURS`.
    pub fn from_env(
        db: DatabaseConnection,
        communication: Arc<CommunicationService>,
    ) -> Result<Self> {
        let steps = parse_steps(
            env::text("PAS_REMINDER_STEPS")
                .as_deref()
                .unwrap_or(DEFAULT_STEPS),
        )?;
        let max =
            usize::try_from(env::int("PAS_REMINDER_MAX_TOUCHES")).unwrap_or(DEFAULT_MAX_TOUCHES);
        Ok(Self::new(db, communication)
            .with_steps(steps)
            .with_max_touches(max)
            .with_link_base(env::text("PAS_LINK_BASE_URL"))
            .with_quiet_hours(super::consent::QuietHours::from_env()?))
    }

    /// Has the patient taken an action: confirmed, asked to reschedule, or used a link? (Never "opened".)
    pub async fn has_acted(&self, appointment_id: Uuid) -> Result<bool> {
        let r = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT (a.confirmed_at IS NOT NULL \
                    OR EXISTS (SELECT 1 FROM inbound_texts t WHERE t.appointment_id = a.id AND t.outcome IN ('applied','needs_staff')) \
                    OR EXISTS (SELECT 1 FROM reply_tokens r WHERE r.appointment_id = a.id AND r.consumed_at IS NOT NULL)) AS acted \
                 FROM appointments a WHERE a.id = $1",
                [appointment_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        Ok(r.and_then(|r| r.try_get::<bool>("", "acted").ok())
            .unwrap_or(false))
    }

    async fn logged(&self, appointment_id: Uuid) -> Result<Vec<(String, String, Option<String>)>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT step, outcome, channel FROM reminder_steps WHERE appointment_id = $1",
                [appointment_id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| Ok((r.col("step")?, r.col("outcome")?, r.col("channel")?)))
            .collect()
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "one log row has this many columns; a struct would only rename them"
    )]
    async fn log(
        &self,
        appointment_id: Uuid,
        step: Step,
        planned_for: DateTime<Utc>,
        channel: Option<DeliveryChannel>,
        locale: Option<&str>,
        outcome: &str,
        letter_id: Option<Uuid>,
    ) -> Result<()> {
        self.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "INSERT INTO reminder_steps (id, appointment_id, step, planned_for, channel, locale, outcome, letter_id) \
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (appointment_id, step) DO NOTHING",
                [
                    Uuid::new_v4().into(),
                    appointment_id.into(),
                    step.as_str().into(),
                    planned_for.into(),
                    channel.map(|c| crate::db::repositories::letter::delivery_channel_to_str(c).to_string()).into(),
                    locale.map(String::from).into(),
                    outcome.into(),
                    letter_id.into(),
                ],
            ))
            .await
            .map_err(Error::Database)?;
        Ok(())
    }

    async fn deliver(
        &self,
        a: &Appointment,
        name: &str,
        step: Step,
        first_channel: Option<DeliveryChannel>,
        now: DateTime<Utc>,
    ) -> Result<Delivery> {
        // A running wording experiment may swap in another variant's template group for this appointment
        // (CAS-7): deterministic, recorded, wording only. Without one this is `name` itself.
        let name = &crate::experiments::resolve_name(&self.db, a.id, name).await?;
        let templates = self.reminders.ensure_templates_named(name).await?;
        let required = super::templates::required_locales();
        let language = crate::contact::preferred_language(&self.db, a.person_id).await?;
        let Some(template) = super::templates::choose(&templates, language.as_deref(), &required)
        else {
            return Ok(Delivery::NoChannel);
        };
        let mut failed = 0;
        for t in &templates {
            failed += LetterRepository::count_failed_for_appointment(&self.db, a.id, t.id).await?;
        }
        if failed >= MAX_FAILED_ATTEMPTS {
            return Ok(Delivery::Failed);
        }
        let lang = crate::inbound::messages::Lang::from_preference(Some(&template.locale));
        let preferred = crate::contact::preferred_channel(&self.db, a.person_id)
            .await?
            .and_then(|c| crate::db::repositories::letter::delivery_channel_from_str(&c).ok());
        let order = channel_order(&template.channels, preferred, step, first_channel);
        let ctx = UserContext::system();

        let mut extra = serde_json::json!({
            "appointment": {
                "id": a.id,
                "start": a.start_datetime.to_rfc3339(),
                "end": a.end_datetime.to_rfc3339(),
                "when": crate::inbound::messages::when(a.start_datetime, lang, crate::inbound::messages::timezone()),
                "reason": a.reason,
            }
        });
        let (mut deferred, mut opted_out) = (false, false);
        let mut link_minted = false;
        for channel in &order {
            if !self.communication.channel_ready(*channel) {
                continue;
            }
            let address = match self
                .communication
                .recipient(a.person_id, *channel, &ctx)
                .await
            {
                Ok(Some(addr)) => addr,
                Ok(None) => continue,
                Err(e) => {
                    tracing::warn!(target: "pas::ladder", appointment_id = %a.id, "recipient lookup failed: {e}");
                    break;
                }
            };
            if super::consent::is_opted_out(&self.db, a.person_id, *channel, Some(&address)).await?
            {
                opted_out = true;
                continue;
            }
            if super::consent::is_intrusive(*channel)
                && self
                    .quiet_hours
                    .is_some_and(|q| q.contains(now, crate::inbound::messages::timezone()))
            {
                deferred = true;
                break;
            }
            // One link per message, minted only when something is about to be sent.
            if !link_minted && let Some(base) = &self.link_base {
                let token =
                    crate::reply_links::mint(&self.db, a.id, a.person_id, a.start_datetime).await?;
                extra["reply_url"] = serde_json::json!(format!("{base}/r/{token}"));
                link_minted = true;
            }
            match self
                .communication
                .generate_letter_for(
                    template.id,
                    a.person_id,
                    Some(a.id),
                    *channel,
                    extra.clone(),
                    &format!("reminder_ladder:{}", step.as_str()),
                    Some(a.id.to_string()),
                    &ctx,
                )
                .await
            {
                Ok(letter) if letter.status == LetterStatus::Sent => {
                    return Ok(Delivery::Sent {
                        channel: *channel,
                        locale: template.locale.clone(),
                        letter_id: letter.id,
                    });
                }
                Ok(_) => continue,
                Err(e) => {
                    tracing::warn!(target: "pas::ladder", appointment_id = %a.id, "send failed: {e}");
                    continue;
                }
            }
        }
        Ok(if deferred {
            Delivery::Deferred
        } else if opted_out {
            Delivery::OptedOut
        } else {
            Delivery::NoChannel
        })
    }

    /// One pass: run the next due step of every appointment that has one.
    #[tracing::instrument(skip_all, fields(job = "ladder"))]
    pub async fn run_once(&self, now: DateTime<Utc>) -> Result<LadderReport> {
        let reach = self
            .steps
            .iter()
            .map(|s| s.before)
            .max()
            .unwrap_or_else(|| Duration::days(7));
        let appointments =
            AppointmentRepository::list_booked_starting_between(&self.db, now, now + reach).await?;
        let mut report = LadderReport {
            considered: appointments.len(),
            ..Default::default()
        };
        for a in appointments {
            let logged = self.logged(a.id).await?;
            let done = |s: Step| logged.iter().any(|(st, _, _)| st == s.as_str());
            let due: Vec<(Step, DateTime<Utc>)> = due_steps(&self.steps, a.start_datetime, now)
                .into_iter()
                .filter(|(s, _)| !done(*s))
                .collect();
            let Some(&(step, planned)) = due.last() else {
                continue;
            };
            // Earlier steps that were never run because the appointment was booked late.
            for (s, at) in &due[..due.len() - 1] {
                self.log(a.id, *s, *at, None, None, "skipped_late", None)
                    .await?;
                report.skipped_late += 1;
            }

            let acted = self.has_acted(a.id).await?;
            match step {
                Step::Second | Step::Call if acted => {
                    self.log(a.id, step, planned, None, None, "skipped_replied", None)
                        .await?;
                    report.skipped_replied += 1;
                }
                Step::Call => {
                    self.db
                        .execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "INSERT INTO staff_call_tasks (id, appointment_id, person_id, reason, due_at) VALUES ($1,$2,$3,'silent',$4) \
                             ON CONFLICT (appointment_id, reason) DO NOTHING",
                            [Uuid::new_v4().into(), a.id.into(), a.person_id.into(), planned.into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    self.log(a.id, step, planned, None, None, "call_task", None)
                        .await?;
                    report.call_tasks += 1;
                }
                _ => {
                    let sent_touches = logged.iter().filter(|(_, o, _)| o == "sent").count();
                    if sent_touches >= self.max_touches {
                        self.log(a.id, step, planned, None, None, "capped", None)
                            .await?;
                        report.capped += 1;
                        continue;
                    }
                    let first_channel = logged
                        .iter()
                        .find(|(s, o, _)| s == "main" && o == "sent")
                        .and_then(|(_, _, c)| c.as_deref())
                        .and_then(|c| {
                            crate::db::repositories::letter::delivery_channel_from_str(c).ok()
                        });
                    let Some(name) = step.template_name() else {
                        continue;
                    };
                    match self.deliver(&a, name, step, first_channel, now).await? {
                        Delivery::Sent {
                            channel,
                            locale,
                            letter_id,
                        } => {
                            self.log(
                                a.id,
                                step,
                                planned,
                                Some(channel),
                                Some(&locale),
                                "sent",
                                Some(letter_id),
                            )
                            .await?;
                            report.sent += 1;
                        }
                        // A held step stays due and goes on a later scan.
                        Delivery::Deferred => report.deferred_quiet_hours += 1,
                        Delivery::OptedOut => {
                            self.log(a.id, step, planned, None, None, "skipped_opted_out", None)
                                .await?;
                            report.skipped_opted_out += 1;
                        }
                        Delivery::NoChannel => {
                            self.log(a.id, step, planned, None, None, "skipped_no_channel", None)
                                .await?;
                            report.skipped_no_channel += 1;
                        }
                        Delivery::Failed => {
                            self.log(a.id, step, planned, None, None, "failed", None)
                                .await?;
                            report.failed += 1;
                        }
                    }
                }
            }
        }
        Ok(report)
    }

    /// Open staff call tasks, soonest first.
    pub async fn open_calls(&self, limit: i64) -> Result<Vec<serde_json::Value>> {
        let rows = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT t.id, t.appointment_id, t.person_id, t.reason, t.due_at, a.start_datetime FROM staff_call_tasks t \
                 JOIN appointments a ON a.id = t.appointment_id WHERE t.handled_at IS NULL ORDER BY a.start_datetime, t.due_at LIMIT $1",
                [crate::paging::fetch(limit).into()],
            ))
            .await
            .map_err(Error::Database)?;
        rows.iter()
            .map(|r| {
                Ok(serde_json::json!({
                    "id": r.col::<Uuid>("id")?,
                    "appointment_id": r.col::<Uuid>("appointment_id")?,
                    "person_id": r.col::<Uuid>("person_id")?,
                    "reason": r.col::<String>("reason")?,
                    "due_at": r.try_get::<DateTime<Utc>>("", "due_at").map_err(Error::Database)?,
                    "appointment_start": r.try_get::<DateTime<Utc>>("", "start_datetime").map_err(Error::Database)?,
                }))
            })
            .collect()
    }

    /// A person phoned: record what happened. Returns false when the task was not open.
    pub async fn complete_call(&self, id: Uuid, outcome: &str, by: Option<&str>) -> Result<bool> {
        if !["reached", "no_answer", "declined_help", "wrong_number"].contains(&outcome) {
            return Err(Error::validation(
                "outcome must be reached, no_answer, declined_help or wrong_number",
            ));
        }
        let r = self
            .db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE staff_call_tasks SET handled_at = now(), handled_by = $3, call_outcome = $2 WHERE id = $1 AND handled_at IS NULL",
                [id.into(), outcome.into(), by.map(String::from).into()],
            ))
            .await
            .map_err(Error::Database)?;
        Ok(r.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DeliveryChannel::*;
    use chrono::TimeZone;

    fn start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 20, 10, 0, 0).unwrap()
    }

    #[test]
    fn test_the_default_ladder_is_seven_four_three_days_and_twenty_four_hours() {
        let s = parse_steps(DEFAULT_STEPS).unwrap();
        assert_eq!(
            s.iter().map(|x| (x.step, x.before)).collect::<Vec<_>>(),
            vec![
                (Step::Main, Duration::days(7)),
                (Step::Second, Duration::days(4)),
                (Step::Call, Duration::days(3)),
                (Step::Final, Duration::hours(24))
            ]
        );
    }

    #[test]
    fn test_bad_step_lists_are_refused() {
        for bad in [
            "",
            "second=4d",
            "main=7d,main=6d",
            "main=4d,second=7d",
            "main=7",
            "main=7w",
            "main=0d",
            "main=61d",
            "wibble=7d",
            "main",
            "main=7d,second=7d",
        ] {
            assert!(parse_steps(bad).is_err(), "{bad:?}");
        }
        assert!(parse_steps("main=7d").is_ok());
        assert!(parse_steps(" main = 10d , final = 2d ").is_ok());
    }

    #[test]
    fn test_steps_become_due_at_their_time_and_not_before() {
        let s = parse_steps(DEFAULT_STEPS).unwrap();
        let at = |d: i64, h: i64| start() - Duration::days(d) - Duration::hours(h);
        assert!(due_steps(&s, start(), at(8, 0)).is_empty());
        assert_eq!(
            due_steps(&s, start(), at(7, 0))
                .iter()
                .map(|x| x.0)
                .collect::<Vec<_>>(),
            vec![Step::Main]
        );
        assert_eq!(
            due_steps(&s, start(), at(4, 0))
                .iter()
                .map(|x| x.0)
                .collect::<Vec<_>>(),
            vec![Step::Main, Step::Second]
        );
        assert_eq!(
            due_steps(&s, start(), at(0, 2)).len(),
            4,
            "a late-booked appointment has them all due; only the latest runs"
        );
        assert_eq!(
            due_steps(&s, start(), at(0, 2)).last().unwrap().0,
            Step::Final
        );
    }

    #[test]
    fn test_the_second_touch_tries_other_channels_with_text_first_and_the_first_channel_last() {
        let offered = [WhatsApp, AppleMessages, Sms, Email];
        // The first message went by WhatsApp: the second tries SMS, then the rest, WhatsApp last.
        assert_eq!(
            channel_order(&offered, Some(WhatsApp), Step::Second, Some(WhatsApp)),
            vec![Sms, AppleMessages, Email, WhatsApp]
        );
        // The main message follows the patient's preference.
        assert_eq!(
            channel_order(&offered, Some(Email), Step::Main, None),
            vec![Email, WhatsApp, AppleMessages, Sms]
        );
        // The final nudge does too.
        assert_eq!(
            channel_order(&offered, Some(Sms), Step::Final, Some(WhatsApp)),
            vec![Sms, WhatsApp, AppleMessages, Email]
        );
        // If the first message never went, the second is just the preference order.
        assert_eq!(
            channel_order(&offered, Some(Sms), Step::Second, None),
            vec![Sms, WhatsApp, AppleMessages, Email]
        );
        // A patient reachable only on the channel already used still gets the second touch there.
        assert_eq!(
            channel_order(&[Sms], None, Step::Second, Some(Sms)),
            vec![Sms]
        );
    }

    #[test]
    fn test_each_sending_step_has_a_template_and_the_call_step_has_none() {
        assert_eq!(Step::Main.template_name(), Some("appointment_reminder"));
        assert!(Step::Second.template_name().is_some() && Step::Final.template_name().is_some());
        assert_eq!(Step::Call.template_name(), None);
    }
}
