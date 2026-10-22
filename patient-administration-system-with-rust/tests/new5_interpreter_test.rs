//! NEW-5: interpreting needs and interpreter requests. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use chrono::{Duration, Utc};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::interpreter::InterpreterService;
use patient_administration_system::models::appointment::CancellationReason;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

async fn requests(db: &sea_orm::DatabaseConnection, appointment: Uuid) -> Vec<(String, String)> {
    db.query_all_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        "SELECT kind, status FROM interpreter_requests WHERE appointment_id = $1",
        [appointment.into()],
    ))
    .await
    .unwrap()
    .iter()
    .map(|r| {
        (
            r.try_get("", "kind").unwrap(),
            r.try_get("", "status").unwrap(),
        )
    })
    .collect()
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn booking_raises_one_request_and_cancelling_cancels_it() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = InterpreterService::new(state.db.clone());
    let schedule = common::new_schedule(&state.db).await.id;
    let needs = Uuid::new_v4();
    let none = Uuid::new_v4();
    svc.set_need(needs, Some("spoken"), Some("pl"), "Receptionist", &sys())
        .await
        .unwrap();

    let slot = common::free_slot(&state.db, schedule, 5).await;
    let a = state
        .scheduling
        .book_slot(slot.id, needs, &sys())
        .await
        .unwrap();
    assert_eq!(
        requests(&state.db, a.id).await,
        vec![("spoken".to_string(), "requested".to_string())]
    );

    let other = common::free_slot(&state.db, schedule, 6).await;
    let b = state
        .scheduling
        .book_slot(other.id, none, &sys())
        .await
        .unwrap();
    assert!(
        requests(&state.db, b.id).await.is_empty(),
        "no need recorded: no request"
    );

    state
        .scheduling
        .cancel(a.id, CancellationReason::PatientRequest, &sys())
        .await
        .unwrap();
    assert_eq!(
        requests(&state.db, a.id).await,
        vec![("spoken".to_string(), "cancelled".to_string())]
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_day_before_list_shows_an_unconfirmed_interpreter_until_someone_confirms_it() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = InterpreterService::new(state.db.clone());
    let schedule = common::new_schedule(&state.db).await.id;
    let person = Uuid::new_v4();
    svc.set_need(person, Some("bsl"), None, "Receptionist", &sys())
        .await
        .unwrap();
    let tomorrow = state
        .scheduling
        .book_slot(
            common::free_slot(&state.db, schedule, 1).await.id,
            person,
            &sys(),
        )
        .await
        .unwrap();
    let later = state
        .scheduling
        .book_slot(
            common::free_slot(&state.db, schedule, 3).await.id,
            person,
            &sys(),
        )
        .await
        .unwrap();

    let listed = |rows: &[patient_administration_system::interpreter::UnconfirmedRequest],
                  id: Uuid| { rows.iter().any(|r| r.appointment_id == id) };
    let soon = svc
        .unconfirmed(Utc::now(), Duration::hours(36), 501)
        .await
        .unwrap();
    assert!(
        listed(&soon, tomorrow.id),
        "tomorrow's unconfirmed interpreter is listed"
    );
    assert!(
        !listed(&soon, later.id),
        "three days ahead is not on the day-before list"
    );

    let request = soon
        .iter()
        .find(|r| r.appointment_id == tomorrow.id)
        .unwrap()
        .request_id;
    assert!(
        svc.confirm(request, " ", None, Utc::now(), &sys())
            .await
            .is_err(),
        "a named person"
    );
    svc.confirm(
        request,
        "Clinic coordinator",
        Some("WITS-12345"),
        Utc::now(),
        &sys(),
    )
    .await
    .unwrap();
    assert!(!listed(
        &svc.unconfirmed(Utc::now(), Duration::hours(36), 501)
            .await
            .unwrap(),
        tomorrow.id
    ));
    assert!(
        svc.confirm(request, "Clinic coordinator", None, Utc::now(), &sys())
            .await
            .is_err(),
        "confirmed once"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_need_says_a_language_only_for_a_spoken_interpreter() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = InterpreterService::new(state.db.clone());
    let p = Uuid::new_v4();
    for (kind, language, by) in [
        (Some("spoken"), None, "Receptionist"),
        (Some("bsl"), Some("en"), "Receptionist"),
        (Some("sign"), None, "Receptionist"),
        (None, Some("pl"), "Receptionist"),
        (Some("deafblind"), None, "  "),
    ] {
        assert!(
            svc.set_need(p, kind, language, by, &sys()).await.is_err(),
            "{kind:?} {language:?} {by:?}"
        );
    }
    svc.set_need(p, Some("deafblind"), None, "Receptionist", &sys())
        .await
        .unwrap();
    svc.set_need(p, None, None, "Receptionist", &sys())
        .await
        .unwrap();
}
