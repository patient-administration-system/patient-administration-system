//! CAP-13: the reminder ladder and the safe reply links. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! Each test places its appointment about a year ahead and runs the ladder with a pretend clock, so the
//! steps are exercised at their real times without waiting and without touching other tests' data.

mod common;

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{DateTime, Duration, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::communication::MessageSender;
use patient_administration_system::communication::consent::{self, QuietHours};
use patient_administration_system::communication::ladder::LadderService;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::{Appointment, AppointmentStatus};
use patient_administration_system::models::communication::DeliveryChannel;
use patient_administration_system::reply_links::{self, LinkAction};
use patient_administration_system::upstream::{Person, StubPersonClient};
use patient_administration_system::writeback::{ReplyAction, ReplyRequest};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

type Log = Arc<Mutex<Vec<(String, String, String)>>>; // (channel, to, body)

struct Capture(&'static str, Log);

#[async_trait]
impl MessageSender for Capture {
    fn is_enabled(&self) -> bool {
        true
    }
    async fn send(
        &self,
        to: &str,
        _s: &str,
        body: &str,
    ) -> patient_administration_system::Result<()> {
        self.1
            .lock()
            .unwrap()
            .push((self.0.to_string(), to.to_string(), body.to_string()));
        Ok(())
    }
}

struct World {
    s: AppState,
    person: Uuid,
    appt: Appointment,
    phone: String,
    email: String,
    log: Log,
}

async fn world() -> World {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let person = Uuid::new_v4();
    let phone = format!("+44 7{:09}", Uuid::new_v4().as_u128() % 1_000_000_000).replace(' ', "");
    let email = format!("{}@example.test", Uuid::new_v4());
    let mut data = serde_json::Map::new();
    data.insert("name".into(), serde_json::json!({ "given": "Ada" }));
    data.insert("telecom".into(), serde_json::json!([{ "system": "phone", "value": phone }, { "system": "email", "value": email }]));
    let stub = Arc::new(StubPersonClient::new());
    stub.insert(Person { id: person, data });
    let s = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(stub)
        .with_sender(DeliveryChannel::Sms, Arc::new(Capture("sms", log.clone())))
        .with_sender(
            DeliveryChannel::WhatsApp,
            Arc::new(Capture("whatsapp", log.clone())),
        )
        .with_sender(
            DeliveryChannel::Email,
            Arc::new(Capture("email", log.clone())),
        );
    let start = Utc::now()
        + Duration::days(365)
        + Duration::minutes((Uuid::new_v4().as_u128() % 100_000) as i64);
    let mut a = Appointment::new(person, start, start + Duration::minutes(30));
    a.status = AppointmentStatus::Booked;
    let appt = AppointmentRepository::create(&s.db, &a).await.unwrap();
    World {
        s,
        person,
        appt,
        phone,
        email,
        log,
    }
}

impl World {
    fn ladder(&self) -> LadderService {
        LadderService::new(self.s.db.clone(), self.s.communication.clone())
            .with_link_base(Some("https://pas.test".into()))
    }
    fn at(&self, before: Duration) -> DateTime<Utc> {
        self.appt.start_datetime - before + Duration::minutes(1)
    }
    fn channels(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, to, _)| *to == self.phone || *to == self.email)
            .map(|(c, _, _)| c.clone())
            .collect()
    }
    async fn steps(&self) -> Vec<(String, String)> {
        let rows = self
            .s
            .db
            .query_all(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT step, outcome FROM reminder_steps WHERE appointment_id = $1 ORDER BY planned_for",
                [self.appt.id.into()],
            ))
            .await
            .unwrap();
        rows.iter()
            .map(|r| {
                (
                    r.try_get("", "step").unwrap(),
                    r.try_get("", "outcome").unwrap(),
                )
            })
            .collect()
    }
    async fn status(&self) -> AppointmentStatus {
        AppointmentRepository::find_by_id(&self.s.db, self.appt.id)
            .await
            .unwrap()
            .unwrap()
            .status
    }
}

const D7: fn() -> Duration = || Duration::days(7);
const D4: fn() -> Duration = || Duration::days(4);
const D3: fn() -> Duration = || Duration::days(3);
const H24: fn() -> Duration = || Duration::hours(24);

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_main_prompt_goes_once_at_seven_days_with_a_link_and_is_logged() {
    let w = world().await;
    let l = w.ladder();
    // Before T-7 nothing happens.
    l.run_once(w.appt.start_datetime - Duration::days(8))
        .await
        .unwrap();
    assert!(w.channels().is_empty() && w.steps().await.is_empty());

    let r = l.run_once(w.at(D7())).await.unwrap();
    assert!(r.sent >= 1, "{r:?}");
    assert_eq!(
        w.steps().await,
        vec![("main".to_string(), "sent".to_string())]
    );
    let body = w
        .log
        .lock()
        .unwrap()
        .iter()
        .find(|(_, to, _)| *to == w.phone)
        .map(|(_, _, b)| b.clone())
        .expect("sent to the phone");
    assert!(body.contains("Can't come?"), "{body}");
    let url = body
        .split_whitespace()
        .find(|t| t.starts_with("https://pas.test/r/"))
        .expect("a link");
    let token = url.trim_start_matches("https://pas.test/r/");
    assert_eq!(token.len(), 64, "an opaque token");
    assert!(
        !url.contains(&w.person.to_string()) && !url.contains(&w.appt.id.to_string()),
        "no personal data in the link"
    );

    // A second scan does not send it again.
    let before = w.channels().len();
    l.run_once(w.at(D7() - Duration::hours(1))).await.unwrap();
    assert_eq!(w.channels().len(), before);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_silent_patient_climbs_the_whole_ladder_whatever_a_score_might_say() {
    let w = world().await;
    let l = w.ladder();
    l.run_once(w.at(D7())).await.unwrap();
    assert_eq!(
        w.channels(),
        vec!["whatsapp"],
        "the main prompt, on the template's first channel"
    );

    // T-4: no reply, so the second touch goes by a different channel, text first.
    l.run_once(w.at(D4())).await.unwrap();
    assert_eq!(w.channels(), vec!["whatsapp", "sms"]);

    // T-3: still silent, so a person is asked to phone.
    l.run_once(w.at(D3())).await.unwrap();
    assert!(
        w.steps()
            .await
            .contains(&("call".to_string(), "call_task".to_string()))
    );
    let calls = l.open_calls(500).await.unwrap();
    let task = calls
        .iter()
        .find(|c| c["appointment_id"] == w.appt.id.to_string())
        .expect("on the call list");
    assert_eq!(task["reason"], "silent");
    // Scanning again does not add a second task.
    l.run_once(w.at(D3() - Duration::hours(2))).await.unwrap();
    assert_eq!(
        l.open_calls(500)
            .await
            .unwrap()
            .iter()
            .filter(|c| c["appointment_id"] == w.appt.id.to_string())
            .count(),
        1
    );

    // T-24h: the final nudge goes to everyone still booked.
    l.run_once(w.at(H24())).await.unwrap();
    assert_eq!(w.channels().len(), 3);
    assert_eq!(
        w.steps().await,
        vec![
            ("main".into(), "sent".into()),
            ("second".into(), "sent".into()),
            ("call".into(), "call_task".into()),
            ("final".into(), "sent".into())
        ]
    );

    // The person phones and records it; it leaves the list.
    let id: Uuid = task["id"].as_str().unwrap().parse().unwrap();
    assert!(
        l.complete_call(id, "reached", Some("nurse-1"))
            .await
            .unwrap()
    );
    assert!(
        !l.complete_call(id, "reached", None).await.unwrap(),
        "only once"
    );
    assert!(
        l.complete_call(id, "shrugged", None).await.is_err(),
        "an unknown outcome is refused"
    );
    assert!(
        !l.open_calls(500)
            .await
            .unwrap()
            .iter()
            .any(|c| c["id"] == id.to_string())
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_patient_who_confirmed_is_not_chased_but_still_gets_the_final_nudge() {
    let w = world().await;
    let l = w.ladder();
    l.run_once(w.at(D7())).await.unwrap();
    w.s.writeback
        .apply(
            ReplyRequest {
                message_id: format!("c-{}", Uuid::new_v4()),
                action: ReplyAction::Confirm,
                appointment_id: w.appt.id,
                person_id: w.person,
                channel: Some("sms".into()),
                barrier_code: None,
                new_slot_id: None,
                received_at: None,
            },
            &UserContext::system(),
        )
        .await
        .unwrap();
    l.run_once(w.at(D4())).await.unwrap();
    assert!(
        w.steps()
            .await
            .contains(&("second".to_string(), "skipped_replied".to_string()))
    );
    l.run_once(w.at(D3())).await.unwrap();
    assert!(
        w.steps()
            .await
            .contains(&("call".to_string(), "skipped_replied".to_string())),
        "no phone call for someone who confirmed"
    );
    assert!(
        !l.open_calls(500)
            .await
            .unwrap()
            .iter()
            .any(|c| c["appointment_id"] == w.appt.id.to_string())
    );
    l.run_once(w.at(H24())).await.unwrap();
    assert_eq!(w.channels().len(), 2, "main and final only");
    assert_eq!(
        w.steps()
            .await
            .iter()
            .map(|(s, o)| format!("{s}:{o}"))
            .collect::<Vec<_>>(),
        vec![
            "main:sent",
            "second:skipped_replied",
            "call:skipped_replied",
            "final:sent"
        ]
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_late_booked_appointment_gets_only_the_latest_due_step() {
    let w = world().await;
    let l = w.ladder();
    // Booked so late that every step is already due: one message, not four.
    l.run_once(w.appt.start_datetime - Duration::hours(2))
        .await
        .unwrap();
    assert_eq!(w.channels().len(), 1);
    assert_eq!(
        w.steps()
            .await
            .iter()
            .filter(|(_, o)| o == "skipped_late")
            .count(),
        3
    );
    assert!(
        w.steps()
            .await
            .contains(&("final".to_string(), "sent".to_string()))
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_touch_cap_stops_further_sends() {
    let w = world().await;
    let l = w.ladder().with_max_touches(1);
    l.run_once(w.at(D7())).await.unwrap();
    l.run_once(w.at(D4())).await.unwrap();
    assert!(
        w.steps()
            .await
            .contains(&("second".to_string(), "capped".to_string()))
    );
    assert_eq!(w.channels().len(), 1);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn opt_outs_stop_every_step_and_quiet_hours_only_hold_one() {
    let w = world().await;
    for ch in ["sms", "whatsapp", "apple_messages", "email"] {
        consent::record_optout(
            &w.s.db,
            &consent::person_subject(w.person),
            ch,
            "staff",
            None,
        )
        .await
        .unwrap();
    }
    let r = w.ladder().run_once(w.at(D7())).await.unwrap();
    assert!(r.skipped_opted_out >= 1);
    assert!(w.channels().is_empty());
    assert_eq!(
        w.steps().await,
        vec![("main".to_string(), "skipped_opted_out".to_string())]
    );

    // Quiet hours: the step is held, not logged, and goes when they end.
    let q = world().await;
    let now = q.at(D7());
    let local = now.with_timezone(&patient_administration_system::inbound::messages::timezone());
    let around = QuietHours::parse(&format!(
        "{}-{}",
        (local - Duration::hours(1)).format("%H:%M"),
        (local + Duration::hours(1)).format("%H:%M")
    ))
    .ok();
    let held = q
        .ladder()
        .with_quiet_hours(around)
        .run_once(now)
        .await
        .unwrap();
    assert!(held.deferred_quiet_hours >= 1, "{held:?}");
    assert!(
        q.channels().is_empty() && q.steps().await.is_empty(),
        "held: nothing sent, nothing logged"
    );
    q.ladder().run_once(now).await.unwrap();
    assert_eq!(
        q.steps().await,
        vec![("main".to_string(), "sent".to_string())]
    );
}

// ---- links ------------------------------------------------------------------------------------

async fn link(w: &World) -> String {
    reply_links::mint(&w.s.db, w.appt.id, w.person, w.appt.start_datetime)
        .await
        .unwrap()
}

fn sys() -> UserContext {
    UserContext::system()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn opening_a_link_changes_nothing_and_only_a_tap_confirms_once() {
    let w = world().await;
    let t = link(&w).await;
    // A scanner or preview opens it many times: nothing happens.
    for _ in 0..5 {
        let i = w.s.reply_links.info(&t).await.unwrap();
        assert_eq!(i.state, "open");
        assert_eq!(i.actions, vec!["confirm", "cancel", "reschedule", "help"]);
        assert!(!i.when.is_empty());
    }
    assert_eq!(w.status().await, AppointmentStatus::Booked);
    assert!(
        AppointmentRepository::find_by_id(&w.s.db, w.appt.id)
            .await
            .unwrap()
            .unwrap()
            .confirmed_at
            .is_none()
    );
    assert!(
        !w.ladder().has_acted(w.appt.id).await.unwrap(),
        "an open is not an action"
    );

    // A deliberate tap confirms.
    let o =
        w.s.reply_links
            .act(&t, LinkAction::Confirm, false, &sys())
            .await
            .unwrap();
    assert_eq!(o.status, "confirmed");
    assert!(
        AppointmentRepository::find_by_id(&w.s.db, w.appt.id)
            .await
            .unwrap()
            .unwrap()
            .confirmed_at
            .is_some()
    );
    assert!(w.ladder().has_acted(w.appt.id).await.unwrap());
    // And only once.
    assert!(
        w.s.reply_links
            .act(&t, LinkAction::Confirm, false, &sys())
            .await
            .is_err()
    );
    assert!(
        w.s.reply_links
            .act(&t, LinkAction::Cancel, true, &sys())
            .await
            .is_err(),
        "a used link cancels nothing"
    );
    assert_eq!(w.s.reply_links.info(&t).await.unwrap().state, "used");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn cancel_asks_are_you_sure_first_and_help_does_not_use_the_link_up() {
    let w = world().await;
    let t = link(&w).await;
    let ask =
        w.s.reply_links
            .act(&t, LinkAction::Cancel, false, &sys())
            .await
            .unwrap();
    assert_eq!(ask.status, "needs_confirmation");
    assert!(ask.message.contains("Are you sure"));
    assert_eq!(
        w.status().await,
        AppointmentStatus::Booked,
        "the first tap changed nothing"
    );
    assert_eq!(
        w.s.reply_links.info(&t).await.unwrap().state,
        "open",
        "and did not use the link"
    );
    let help =
        w.s.reply_links
            .act(&t, LinkAction::Help, false, &sys())
            .await
            .unwrap();
    assert_eq!(help.status, "help");
    assert_eq!(w.s.reply_links.info(&t).await.unwrap().state, "open");
    let done =
        w.s.reply_links
            .act(&t, LinkAction::Cancel, true, &sys())
            .await
            .unwrap();
    assert_eq!(done.status, "cancelled");
    assert_eq!(w.status().await, AppointmentStatus::Cancelled);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn reschedule_asks_for_a_person_and_leaves_the_appointment_alone() {
    let w = world().await;
    let t = link(&w).await;
    let o =
        w.s.reply_links
            .act(&t, LinkAction::Reschedule, false, &sys())
            .await
            .unwrap();
    assert_eq!(o.status, "reschedule_requested");
    assert_eq!(w.status().await, AppointmentStatus::Booked);
    let calls = w.s.ladder.open_calls(500).await.unwrap();
    assert!(calls.iter().any(
        |c| c["appointment_id"] == w.appt.id.to_string() && c["reason"] == "reschedule_request"
    ));
    assert!(
        w.ladder().has_acted(w.appt.id).await.unwrap(),
        "a reschedule request is an action: no chasing"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn bad_expired_and_double_tapped_links_are_refused() {
    let w = world().await;
    // Unknown, malformed and empty tokens look alike: "not valid".
    for bad in ["", "abc", &"0".repeat(64), &"g".repeat(64), &"a".repeat(65)] {
        assert!(w.s.reply_links.info(bad).await.is_err(), "{bad:?}");
        assert!(
            w.s.reply_links
                .act(bad, LinkAction::Confirm, false, &sys())
                .await
                .is_err()
        );
    }
    // Expired.
    let old = reply_links::mint(
        &w.s.db,
        w.appt.id,
        w.person,
        Utc::now() - Duration::minutes(1),
    )
    .await
    .unwrap();
    assert_eq!(w.s.reply_links.info(&old).await.unwrap().state, "expired");
    assert!(
        w.s.reply_links
            .act(&old, LinkAction::Confirm, false, &sys())
            .await
            .is_err()
    );
    // Only a hash is stored: the token itself is nowhere in the database.
    let t = link(&w).await;
    let plain =
        w.s.db
            .query_one(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*) AS n FROM reply_tokens WHERE token_hash = $1",
                [t.clone().into()],
            ))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(plain.try_get::<i64>("", "n").unwrap(), 0);
    // Two taps at the same instant act once.
    let ctx = sys();
    let (a, b) = tokio::join!(
        w.s.reply_links.act(&t, LinkAction::Confirm, false, &ctx),
        w.s.reply_links.act(&t, LinkAction::Confirm, false, &ctx)
    );
    assert_eq!(
        [a.is_ok(), b.is_ok()].iter().filter(|x| **x).count(),
        1,
        "exactly one tap wins"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_read_without_acting_and_act_on_post() {
    let w = world().await;
    let t = link(&w).await;
    let app = patient_administration_system::api::rest::router(w.s.clone());
    let call = |method: &'static str, uri: String, body: Option<serde_json::Value>| {
        let app = app.clone();
        async move {
            let mut b = Request::builder().method(method).uri(uri);
            let body = match body {
                Some(v) => {
                    b = b.header("content-type", "application/json");
                    Body::from(v.to_string())
                }
                None => Body::empty(),
            };
            let r = app.oneshot(b.body(body).unwrap()).await.unwrap();
            let st = r.status();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(
                    &to_bytes(r.into_body(), 1 << 20).await.unwrap(),
                )
                .unwrap_or_default(),
            )
        }
    };
    let (st, v) = call("GET", format!("/api/reply-links/{t}"), None).await;
    assert_eq!(
        (st, v["data"]["state"].as_str()),
        (StatusCode::OK, Some("open"))
    );
    // GET on an action path does not act (only POST is routed).
    let (st, _) = call("GET", format!("/api/reply-links/{t}/confirm"), None).await;
    assert_eq!(st, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(w.status().await, AppointmentStatus::Booked);
    assert_eq!(
        call("POST", format!("/api/reply-links/{t}/frobnicate"), None)
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call("GET", format!("/api/reply-links/{}", "0".repeat(64)), None)
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let (st, v) = call(
        "POST",
        format!("/api/reply-links/{t}/cancel"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(
        (st, v["data"]["status"].as_str()),
        (StatusCode::OK, Some("needs_confirmation"))
    );
    let (st, v) = call(
        "POST",
        format!("/api/reply-links/{t}/cancel"),
        Some(serde_json::json!({ "confirmed": true })),
    )
    .await;
    assert_eq!(
        (st, v["data"]["status"].as_str()),
        (StatusCode::OK, Some("cancelled"))
    );
    assert_eq!(
        call(
            "POST",
            format!("/api/reply-links/{t}/cancel"),
            Some(serde_json::json!({ "confirmed": true }))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );

    // The staff call list endpoints.
    let (st, v) = call("GET", "/api/staff-call-tasks/open".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    assert!(v["data"].is_array());
    assert_eq!(
        call(
            "POST",
            format!("/api/staff-call-tasks/{}/outcome", Uuid::new_v4()),
            Some(serde_json::json!({ "outcome": "reached" }))
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
}
