//! CAP-7: a person follows up a missed appointment on a listed service.
//! Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::repositories::waitlist::WaitlistRepository;
use patient_administration_system::models::appointment::Appointment;
use patient_administration_system::models::waitlist::{Priority, WaitlistStatus};
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

struct W {
    s: AppState,
    service: String,
    schedule: Uuid,
}

async fn world() -> W {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("clinic-{}", Uuid::new_v4());
    let schedule = common::new_schedule_for(&s.db, &service).await.id;
    W {
        s,
        service,
        schedule,
    }
}

impl W {
    async fn book(&self, person: Uuid) -> Appointment {
        let slot = common::free_slot(&self.s.db, self.schedule, 3).await;
        self.s
            .scheduling
            .book_slot(slot.id, person, &sys())
            .await
            .unwrap()
    }
    async fn list(&self) {
        self.s
            .follow_up
            .approve_policy(&self.service, "Dr A. Clinician", &sys())
            .await
            .unwrap();
    }
    async fn tasks_for(&self, appt: Uuid) -> i64 {
        let r = self
            .s
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*) AS n FROM follow_up_tasks WHERE appointment_id = $1",
                [appt.into()],
            ))
            .await
            .unwrap()
            .unwrap();
        r.try_get("", "n").unwrap()
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_policy_without_an_approver_is_refused() {
    let w = world().await;
    for blank in ["", "  "] {
        assert!(
            w.s.follow_up
                .approve_policy(&w.service, blank, &sys())
                .await
                .is_err()
        );
    }
    assert!(
        w.s.follow_up
            .policies()
            .await
            .unwrap()
            .iter()
            .all(|p| p.service_type != w.service)
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_no_show_on_a_listed_service_creates_one_urgent_task() {
    let w = world().await;
    w.list().await;
    let a = w.book(Uuid::new_v4()).await;
    w.s.scheduling.mark_no_show(a.id, &sys()).await.unwrap();
    assert_eq!(w.tasks_for(a.id).await, 1);
    let open = w.s.follow_up.open_tasks(500).await.unwrap();
    let t = open
        .iter()
        .find(|t| t.appointment_id == a.id)
        .expect("task is open");
    assert_eq!(t.person_id, a.person_id);
    assert_eq!(t.service_type, w.service);
    // Due within 48 hours of the miss.
    let hours = (t.due_at - chrono::Utc::now()).num_hours();
    assert!((46..=48).contains(&hours), "due in {hours} hours");
    assert_eq!(
        common::audit_count(&w.s.db, a.id, "follow_up_task_created").await,
        1
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_no_show_on_an_unlisted_service_creates_no_task() {
    let w = world().await;
    let a = w.book(Uuid::new_v4()).await;
    w.s.scheduling.mark_no_show(a.id, &sys()).await.unwrap();
    assert_eq!(w.tasks_for(a.id).await, 0);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn only_a_person_recording_an_outcome_closes_a_task() {
    let w = world().await;
    w.list().await;
    let a = w.book(Uuid::new_v4()).await;
    w.s.scheduling.mark_no_show(a.id, &sys()).await.unwrap();
    let id =
        w.s.follow_up
            .open_tasks(500)
            .await
            .unwrap()
            .into_iter()
            .find(|t| t.appointment_id == a.id)
            .unwrap()
            .id;
    // Time passing, a reminder, a rebooking: none of them close it.
    assert!(
        w.s.follow_up
            .open_tasks(500)
            .await
            .unwrap()
            .iter()
            .any(|t| t.id == id)
    );
    // No outcome, a made-up outcome, or no name: refused.
    assert!(
        w.s.follow_up
            .close_task(id, "", "Nurse B", &sys())
            .await
            .is_err()
    );
    assert!(
        w.s.follow_up
            .close_task(id, "discharged", "Nurse B", &sys())
            .await
            .is_err()
    );
    assert!(
        w.s.follow_up
            .close_task(id, "reached_will_attend", " ", &sys())
            .await
            .is_err()
    );
    assert!(
        w.s.follow_up
            .open_tasks(500)
            .await
            .unwrap()
            .iter()
            .any(|t| t.id == id)
    );

    w.s.follow_up
        .close_task(id, "reached_will_attend", "Nurse B", &sys())
        .await
        .unwrap();
    assert!(
        w.s.follow_up
            .open_tasks(500)
            .await
            .unwrap()
            .iter()
            .all(|t| t.id != id)
    );
    assert!(
        w.s.follow_up
            .close_task(id, "no_answer", "Nurse C", &sys())
            .await
            .is_err(),
        "closed once"
    );
    assert_eq!(
        common::audit_count(&w.s.db, a.id, "follow_up_task_closed").await,
        1
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_miss_never_removes_the_patient_or_touches_their_rtt_clock() {
    let w = world().await;
    w.list().await;
    let person = Uuid::new_v4();
    let entry =
        w.s.waitlist
            .add(None, person, w.service.clone(), Priority::Urgent, &sys())
            .await
            .unwrap();
    let pathway =
        w.s.rtt
            .start(person, w.service.clone(), &sys())
            .await
            .unwrap();
    let events = |db: sea_orm::DatabaseConnection, id: Uuid| async move {
        let r = db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT count(*) AS n FROM rtt_clock_events WHERE pathway_id = $1",
                [id.into()],
            ))
            .await
            .unwrap()
            .unwrap();
        r.try_get::<i64>("", "n").unwrap()
    };
    let before = events(w.s.db.clone(), pathway.id).await;

    let slot = common::free_slot(&w.s.db, w.schedule, 3).await;
    let a =
        w.s.scheduling
            .book_for_entry(entry.id, slot.id, &sys())
            .await
            .unwrap();
    w.s.scheduling.mark_no_show(a.id, &sys()).await.unwrap();

    let e = WaitlistRepository::find_entry_by_id(&w.s.db, entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        e.status,
        WaitlistStatus::Waiting,
        "back on the list, not removed"
    );
    assert_eq!(
        events(w.s.db.clone(), pathway.id).await,
        before,
        "the clock is untouched"
    );
    assert!(!w.s.rtt.is_breaching(pathway.id).await.unwrap());
    assert_eq!(
        w.tasks_for(a.id).await,
        1,
        "and a person is asked to make contact"
    );

    // This patient is not in the patients table, and the RTT cockpit lists every open pathway, so do not
    // leave it behind for the other suites to trip over.
    for sql in [
        "DELETE FROM rtt_clock_events WHERE pathway_id = $1",
        "DELETE FROM rtt_pathways WHERE id = $1",
    ] {
        w.s.db
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                sql,
                [pathway.id.into()],
            ))
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_endpoints_list_policies_and_close_tasks() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let w = world().await;
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
    let uri = format!("/api/follow-up-policies/{}", w.service);
    let (st, _) = call(
        "PUT",
        uri.clone(),
        Some(serde_json::json!({ "approved_by": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, v) = call(
        "PUT",
        uri,
        Some(serde_json::json!({ "approved_by": "Dr A" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let policy: patient_administration_system::follow_up::Policy =
        serde_json::from_value(v["data"].clone()).unwrap();
    assert_eq!(policy.service_type, w.service);

    let a = w.book(Uuid::new_v4()).await;
    w.s.scheduling.mark_no_show(a.id, &sys()).await.unwrap();
    let (st, v) = call("GET", "/api/follow-up-tasks/open?limit=500".into(), None).await;
    assert_eq!(st, StatusCode::OK);
    let id = v["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["appointment_id"] == a.id.to_string())
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let close = format!("/api/follow-up-tasks/{id}/outcome");
    let (st, _) = call(
        "POST",
        close.clone(),
        Some(serde_json::json!({ "outcome": "reached_rebooked", "closed_by": "" })),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let (st, _) = call(
        "POST",
        close.clone(),
        Some(serde_json::json!({ "outcome": "reached_rebooked", "closed_by": "Nurse B" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let (st, _) = call(
        "POST",
        close,
        Some(serde_json::json!({ "outcome": "no_answer", "closed_by": "Nurse B" })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_patients_cancel_on_a_listed_service_is_held_for_a_person_to_phone_first() {
    use patient_administration_system::models::appointment::AppointmentStatus;
    use patient_administration_system::models::schedule::SlotStatus;
    use patient_administration_system::reply_links::{self, LinkAction};
    use patient_administration_system::writeback::{
        HELD_FOR_STAFF, ReplyAction, ReplyRequest, ReplyStatus,
    };
    let w = world().await;
    w.list().await;
    let person = Uuid::new_v4();
    let a = w.book(person).await;
    let req = ReplyRequest {
        message_id: format!("m-{}", Uuid::new_v4()),
        action: ReplyAction::Cancel,
        appointment_id: a.id,
        person_id: person,
        channel: Some("sms".into()),
        barrier_code: None,
        new_slot_id: None,
        modality: None,
        received_at: None,
    };
    let r = w.s.writeback.apply(req, &sys()).await.unwrap();
    assert_eq!(r.status, ReplyStatus::Rejected);
    assert_eq!(r.reject_reason.as_deref(), Some(HELD_FOR_STAFF));
    let now = patient_administration_system::db::repositories::appointment::AppointmentRepository::find_by_id(&w.s.db, a.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(now.status, AppointmentStatus::Booked, "not cancelled");
    let slot = patient_administration_system::db::repositories::slot::SlotRepository::find_by_id(
        &w.s.db,
        a.slot_id.unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(slot.status, SlotStatus::Busy, "the slot is not released");
    let calls = w.s.ladder.open_calls(500).await.unwrap();
    assert!(
        calls
            .iter()
            .any(|c| c["appointment_id"] == a.id.to_string() && c["reason"] == "urgent_pathway"),
        "a person is asked to phone"
    );

    // The same through a reply link: the second, confirmed tap is held too, and says so.
    let b = w.book(Uuid::new_v4()).await;
    let t = reply_links::mint(&w.s.db, b.id, b.person_id, b.start_datetime)
        .await
        .unwrap();
    let o =
        w.s.reply_links
            .act(&t, LinkAction::Cancel, true, &sys())
            .await
            .unwrap();
    assert_eq!(o.status, "held_for_call");
    assert!(
        o.message.contains("someone will phone you"),
        "{}",
        o.message
    );

    // A clinician's own cancel is not held.
    w.s.scheduling
        .cancel(
            a.id,
            patient_administration_system::models::appointment::CancellationReason::ProviderRequest,
            &sys(),
        )
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_cancel_on_an_unlisted_service_goes_through() {
    use patient_administration_system::writeback::{ReplyAction, ReplyRequest, ReplyStatus};
    let w = world().await;
    let person = Uuid::new_v4();
    let a = w.book(person).await;
    let req = ReplyRequest {
        message_id: format!("m-{}", Uuid::new_v4()),
        action: ReplyAction::Cancel,
        appointment_id: a.id,
        person_id: person,
        channel: Some("sms".into()),
        barrier_code: None,
        new_slot_id: None,
        modality: None,
        received_at: None,
    };
    let r = w.s.writeback.apply(req, &sys()).await.unwrap();
    assert_eq!(r.status, ReplyStatus::Applied);
}
