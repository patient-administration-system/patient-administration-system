//! CAP-5: patient-led rebooking. Needs PostgreSQL: `cargo test -- --ignored`.
//!
//! Each test uses its own schedule and patient, so the offers are its own.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::appointment::AppointmentRepository;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::slot::SlotRepository;
use patient_administration_system::models::appointment::{
    Appointment, AppointmentStatus, CancellationReason,
};
use patient_administration_system::models::schedule::{Slot, SlotStatus};
use patient_administration_system::reply_links::{self, LinkAction};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

/// `days` ahead at `hour`:00 UTC.
fn at(days: i64, hour: u32) -> DateTime<Utc> {
    let d = (Utc::now() + Duration::days(days)).date_naive();
    Utc.with_ymd_and_hms(d.year(), d.month(), d.day(), hour, 0, 0)
        .unwrap()
}

struct W {
    s: AppState,
    schedule: Uuid,
    person: Uuid,
    appt: Appointment,
}

async fn slot(s: &AppState, schedule: Uuid, start: DateTime<Utc>, mins: i64) -> Slot {
    SlotRepository::create(
        &s.db,
        &Slot::new(schedule, start, start + Duration::minutes(mins)),
    )
    .await
    .unwrap()
}

/// A patient booked into day +5 at 09:00, with an empty schedule around them.
async fn world() -> W {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let schedule = common::new_schedule(&s.db).await.id;
    let person = Uuid::new_v4();
    let own = slot(&s, schedule, at(5, 9), 15).await;
    let appt = s
        .scheduling
        .book_slot(own.id, person, &sys())
        .await
        .unwrap();
    W {
        s,
        schedule,
        person,
        appt,
    }
}

impl W {
    async fn add(&self, days: i64, hour: u32) -> Slot {
        slot(&self.s, self.schedule, at(days, hour), 15).await
    }
    async fn status(&self, id: Uuid) -> AppointmentStatus {
        AppointmentRepository::find_by_id(&self.s.db, id)
            .await
            .unwrap()
            .unwrap()
            .status
    }
    async fn slot_status(&self, id: Uuid) -> SlotStatus {
        SlotRepository::find_by_id(&self.s.db, id)
            .await
            .unwrap()
            .unwrap()
            .status
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_offer_is_three_real_times_spread_across_days_and_the_day() {
    let w = world().await;
    // Three mornings on one day first, then an afternoon and an evening on later days.
    let m1 = w.add(6, 9).await;
    let _m2 = w.add(6, 10).await;
    let _m3 = w.add(6, 11).await;
    let a = w.add(7, 14).await;
    let e = w.add(8, 20).await;

    let o = w.s.rebooking.offer(w.appt.id).await.unwrap();
    let got: Vec<Uuid> = o.options.iter().map(|x| x.slot_id).collect();
    assert_eq!(
        got,
        vec![m1.id, a.id, e.id],
        "different days and parts of the day"
    );
    assert!(o.offer_id.is_some());

    // Asking again while the offer stands gives the same offer, so a refresh does not move the choices.
    let again = w.s.rebooking.offer(w.appt.id).await.unwrap();
    assert_eq!(again.offer_id, o.offer_id);
    assert_eq!(
        again.options.iter().map(|x| x.slot_id).collect::<Vec<_>>(),
        got
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn options_are_never_clashing_taken_too_soon_too_far_or_the_wrong_length() {
    let w = world().await;
    // The patient has another appointment on day +6 at 09:00.
    let other_start = at(6, 9);
    let mut other = Appointment::new(w.person, other_start, other_start + Duration::minutes(30));
    other.status = AppointmentStatus::Booked;
    AppointmentRepository::create(&w.s.db, &other)
        .await
        .unwrap();

    let clash = w.add(6, 9).await;
    let busy = w.add(6, 12).await;
    SlotRepository::update_status(&w.s.db, busy.id, SlotStatus::Busy)
        .await
        .unwrap();
    let blocked = w.add(6, 13).await;
    SlotRepository::update_status(&w.s.db, blocked.id, SlotStatus::BlockedOut)
        .await
        .unwrap();
    let too_soon = slot(&w.s, w.schedule, Utc::now() + Duration::hours(2), 15).await;
    let too_far = w.add(400, 9).await;
    let long = slot(&w.s, w.schedule, at(9, 10), 45).await;
    let elsewhere_schedule = common::new_schedule(&w.s.db).await.id;
    let elsewhere = slot(&w.s, elsewhere_schedule, at(9, 11), 15).await;
    let good = w.add(10, 15).await;

    let o = w.s.rebooking.offer(w.appt.id).await.unwrap();
    let got: Vec<Uuid> = o.options.iter().map(|x| x.slot_id).collect();
    assert_eq!(got, vec![good.id]);
    for bad in [
        clash.id,
        busy.id,
        blocked.id,
        too_soon.id,
        too_far.id,
        long.id,
        elsewhere.id,
    ] {
        assert!(!got.contains(&bad), "{bad} must not be offered");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_time_that_was_not_offered_is_refused_and_nothing_changes() {
    let w = world().await;
    let a = w.add(6, 9).await;
    let b = w.add(7, 14).await;
    let c = w.add(8, 20).await;
    let extra = w.add(9, 9).await; // free, same schedule, but beyond the three offered
    let o = w.s.rebooking.offer(w.appt.id).await.unwrap();
    assert_eq!(o.options.len(), 3);
    let offer = o.offer_id.unwrap();

    let err =
        w.s.rebooking
            .choose(w.appt.id, offer, extra.id, &sys())
            .await
            .unwrap_err();
    assert!(
        err.to_string().contains("not one of the times offered"),
        "{err}"
    );
    assert_eq!(w.status(w.appt.id).await, AppointmentStatus::Booked);
    assert_eq!(w.slot_status(extra.id).await, SlotStatus::Free);
    for s in [&a, &b, &c] {
        assert_eq!(w.slot_status(s.id).await, SlotStatus::Free);
    }
    // An offer made for another appointment cannot be used on this one.
    assert!(
        w.s.rebooking
            .choose(Uuid::new_v4(), offer, a.id, &sys())
            .await
            .is_err()
    );
    assert_eq!(w.status(w.appt.id).await, AppointmentStatus::Booked);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn choosing_an_offered_time_moves_the_patient_once_and_atomically() {
    let w = world().await;
    let a = w.add(6, 9).await;
    let _b = w.add(7, 14).await;
    let o = w.s.rebooking.offer(w.appt.id).await.unwrap();
    let offer = o.offer_id.unwrap();

    let (old, new) =
        w.s.rebooking
            .choose(w.appt.id, offer, a.id, &sys())
            .await
            .unwrap();
    assert_eq!(old.status, AppointmentStatus::Cancelled);
    assert_eq!(
        old.cancellation_reason,
        Some(CancellationReason::Rescheduled)
    );
    assert_eq!(new.status, AppointmentStatus::Booked);
    assert_eq!(new.slot_id, Some(a.id));
    assert_eq!(new.person_id, w.person);
    assert_eq!(w.slot_status(a.id).await, SlotStatus::Busy);
    // The old slot is free again for someone else.
    assert_eq!(
        w.slot_status(w.appt.slot_id.unwrap()).await,
        SlotStatus::Free
    );
    assert_eq!(
        common::audit_count(&w.s.db, old.id, "rebook_offer_chosen").await,
        1
    );

    // The offer is used up.
    let again =
        w.s.rebooking
            .choose(w.appt.id, offer, a.id, &sys())
            .await
            .unwrap_err();
    assert!(again.to_string().contains("already been used"), "{again}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_slot_taken_after_the_offer_leaves_the_patient_where_they_were() {
    let w = world().await;
    let a = w.add(6, 9).await;
    let o = w.s.rebooking.offer(w.appt.id).await.unwrap();
    let offer = o.offer_id.unwrap();
    // Someone else takes it first.
    w.s.scheduling
        .book_slot(a.id, Uuid::new_v4(), &sys())
        .await
        .unwrap();

    assert!(
        w.s.rebooking
            .choose(w.appt.id, offer, a.id, &sys())
            .await
            .is_err()
    );
    assert_eq!(
        w.status(w.appt.id).await,
        AppointmentStatus::Booked,
        "still booked"
    );
    // A fresh ask does not offer the taken slot.
    let fresh = w.s.rebooking.offer(w.appt.id).await.unwrap();
    assert!(fresh.options.iter().all(|x| x.slot_id != a.id));
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_expired_offer_and_a_cancelled_appointment_are_refused() {
    let w = world().await;
    let a = w.add(6, 9).await;
    let o = w.s.rebooking.offer(w.appt.id).await.unwrap();
    w.s.db
        .execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE rebook_offers SET expires_at = now() - interval '1 minute' WHERE id = $1",
            [o.offer_id.unwrap().into()],
        ))
        .await
        .unwrap();
    let err =
        w.s.rebooking
            .choose(w.appt.id, o.offer_id.unwrap(), a.id, &sys())
            .await
            .unwrap_err();
    assert!(err.to_string().contains("expired"), "{err}");

    // A cancelled appointment cannot be offered anything.
    w.s.scheduling
        .cancel(w.appt.id, CancellationReason::PatientRequest, &sys())
        .await
        .unwrap();
    assert!(w.s.rebooking.offer(w.appt.id).await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn nothing_to_offer_is_an_empty_offer_not_an_error() {
    let w = world().await;
    let o = w.s.rebooking.offer(w.appt.id).await.unwrap();
    assert!(o.options.is_empty());
    assert!(o.offer_id.is_none());
}

async fn link(w: &W) -> String {
    reply_links::mint(&w.s.db, w.appt.id, w.person, w.appt.start_datetime)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn reschedule_on_a_link_offers_times_without_using_the_link_then_one_choice_books() {
    let w = world().await;
    let a = w.add(6, 9).await;
    let b = w.add(7, 14).await;
    let t = link(&w).await;

    let o =
        w.s.reply_links
            .act(&t, LinkAction::Reschedule, false, &sys())
            .await
            .unwrap();
    assert_eq!(o.status, "rebook_options");
    assert_eq!(o.options.len(), 2);
    assert!(o.options.iter().any(|x| x.slot_id == a.id));
    // Looking is not acting: the link is still open and the appointment stands.
    assert_eq!(w.s.reply_links.info(&t).await.unwrap().state, "open");
    assert_eq!(w.status(w.appt.id).await, AppointmentStatus::Booked);
    // Asking again shows the same two times.
    let o2 =
        w.s.reply_links
            .act(&t, LinkAction::Reschedule, false, &sys())
            .await
            .unwrap();
    assert_eq!(
        o2.options.iter().map(|x| x.slot_id).collect::<Vec<_>>(),
        o.options.iter().map(|x| x.slot_id).collect::<Vec<_>>()
    );

    // A slot that was not on offer is refused and the link is still usable.
    let stray = w.add(9, 9).await;
    assert!(w.s.reply_links.choose(&t, stray.id, &sys()).await.is_err());
    assert_eq!(w.s.reply_links.info(&t).await.unwrap().state, "open");

    let done = w.s.reply_links.choose(&t, b.id, &sys()).await.unwrap();
    assert_eq!(done.status, "rebooked");
    assert_eq!(w.status(w.appt.id).await, AppointmentStatus::Cancelled);
    assert_eq!(w.slot_status(b.id).await, SlotStatus::Busy);
    assert_eq!(w.s.reply_links.info(&t).await.unwrap().state, "used");
    assert!(w.s.reply_links.choose(&t, a.id, &sys()).await.is_err());
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn reschedule_with_no_times_to_offer_goes_to_a_person_as_before() {
    let w = world().await;
    let t = link(&w).await;
    let o =
        w.s.reply_links
            .act(&t, LinkAction::Reschedule, false, &sys())
            .await
            .unwrap();
    assert_eq!(o.status, "reschedule_requested");
    assert_eq!(w.s.reply_links.info(&t).await.unwrap().state, "used");
    assert_eq!(
        w.status(w.appt.id).await,
        AppointmentStatus::Booked,
        "the appointment stands"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_slot_lost_between_offer_and_tap_gives_the_link_back() {
    let w = world().await;
    let a = w.add(6, 9).await;
    let t = link(&w).await;
    w.s.reply_links
        .act(&t, LinkAction::Reschedule, false, &sys())
        .await
        .unwrap();
    w.s.scheduling
        .book_slot(a.id, Uuid::new_v4(), &sys())
        .await
        .unwrap();
    assert!(w.s.reply_links.choose(&t, a.id, &sys()).await.is_err());
    assert_eq!(
        w.s.reply_links.info(&t).await.unwrap().state,
        "open",
        "the patient can ask again"
    );
    assert_eq!(w.status(w.appt.id).await, AppointmentStatus::Booked);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_offer_and_rebook() {
    let w = world().await;
    let a = w.add(6, 9).await;
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
    let (st, v) = call(
        "GET",
        format!("/api/appointments/{}/rebook-options", w.appt.id),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let made: patient_administration_system::rebooking::Offer =
        serde_json::from_value(v["data"].clone()).unwrap();
    let offer = made.offer_id.unwrap().to_string();
    assert_eq!(v["data"]["options"][0]["slot_id"], a.id.to_string());

    let (st, _) = call(
        "POST",
        format!("/api/appointments/{}/rebook", w.appt.id),
        Some(serde_json::json!({ "offer_id": offer, "slot_id": Uuid::new_v4() })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "a slot not offered");
    let (st, v) = call(
        "POST",
        format!("/api/appointments/{}/rebook", w.appt.id),
        Some(serde_json::json!({ "offer_id": offer, "slot_id": a.id })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let done: patient_administration_system::api::rest::handlers_scheduling::Rebooked =
        serde_json::from_value(v["data"].clone()).unwrap();
    assert_eq!(done.new.slot_id, Some(a.id));
    let (st, _) = call(
        "POST",
        format!("/api/appointments/{}/rebook", w.appt.id),
        Some(serde_json::json!({ "offer_id": offer, "slot_id": a.id })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT, "used once");
}
