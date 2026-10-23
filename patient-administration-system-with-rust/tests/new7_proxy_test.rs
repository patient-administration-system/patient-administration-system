//! NEW-7: carer and proxy authority. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use chrono::{Duration, Utc};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::models::appointment::CancelledBy;
use patient_administration_system::proxy::{NewAuthority, ProxyService, proxy_context};
use patient_administration_system::scheduling::CancelDetails;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use uuid::Uuid;

fn scopes(s: &[&str]) -> Vec<String> {
    s.iter().map(|x| x.to_string()).collect()
}

fn new<'a>(proxy: Uuid, scopes: &'a [String], days: i64) -> NewAuthority<'a> {
    NewAuthority {
        proxy_person_id: proxy,
        relationship: "daughter",
        basis: "patient_consent",
        scopes,
        expires_at: Utc::now() + Duration::days(days),
        recorded_by: "Receptionist",
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_proxy_without_the_cancel_scope_cannot_cancel_and_one_with_it_is_audited_by_name() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = ProxyService::new(state.db.clone());
    let sys = UserContext::system();
    let schedule = common::new_schedule(&state.db).await.id;
    let (patient, reader, canceller) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let now = Utc::now();
    svc.grant(patient, &new(reader, &scopes(&["receive"]), 30), now, &sys)
        .await
        .unwrap();
    let can = svc
        .grant(
            patient,
            &new(canceller, &scopes(&["receive", "cancel"]), 30),
            now,
            &sys,
        )
        .await
        .unwrap();

    assert!(svc.authorise(patient, reader, "receive", now).await.is_ok());
    assert!(
        svc.authorise(patient, reader, "cancel", now).await.is_err(),
        "a proxy without the cancel scope cannot cancel"
    );
    assert!(
        svc.authorise(patient, Uuid::new_v4(), "receive", now)
            .await
            .is_err(),
        "a stranger has nothing"
    );

    let appt = state
        .scheduling
        .book_slot(
            common::free_slot(&state.db, schedule, 5).await.id,
            patient,
            &sys,
        )
        .await
        .unwrap();
    let authority = svc
        .authorise(patient, canceller, "cancel", now)
        .await
        .unwrap();
    assert_eq!(authority.id, can.id);
    state
        .scheduling
        .cancel_with(
            appt.id,
            CancelDetails {
                cancelled_by: CancelledBy::Patient,
                barrier_code: None,
                legacy_reason: None,
            },
            &proxy_context(&sys, &authority),
        )
        .await
        .unwrap();

    let who: String = state
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "SELECT user_id FROM audit_log WHERE entity_id = $1 AND action ILIKE '%cancel%' ORDER BY at DESC LIMIT 1",
            [appt.id.into()],
        ))
        .await
        .unwrap()
        .expect("the cancellation was audited")
        .try_get("", "user_id")
        .unwrap();
    assert!(who.starts_with("proxy:"), "{who}");
    assert!(
        who.contains(&canceller.to_string()),
        "names the proxy: {who}"
    );
    assert!(
        who.contains(&patient.to_string()),
        "names the patient: {who}"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_expired_or_revoked_proxy_can_do_nothing_and_bad_grants_are_refused() {
    let state = common::build_state(&common::database_url().unwrap()).await;
    let svc = ProxyService::new(state.db.clone());
    let sys = UserContext::system();
    let (patient, proxy) = (Uuid::new_v4(), Uuid::new_v4());
    let now = Utc::now();
    let all = scopes(&["receive", "confirm", "cancel", "rebook"]);
    let a = svc
        .grant(patient, &new(proxy, &all, 10), now, &sys)
        .await
        .unwrap();

    // Inside the term every scope works; at the expiry none does.
    for s in ["receive", "confirm", "cancel", "rebook"] {
        assert!(svc.authorise(patient, proxy, s, now).await.is_ok(), "{s}");
        assert!(
            svc.authorise(patient, proxy, s, a.expires_at)
                .await
                .is_err(),
            "{s} after expiry"
        );
    }
    // Revoking ends it at once, once only.
    svc.revoke(a.id, "Receptionist", now, &sys).await.unwrap();
    assert!(svc.authorise(patient, proxy, "receive", now).await.is_err());
    assert!(svc.revoke(a.id, "Receptionist", now, &sys).await.is_err());
    assert!(
        svc.for_patient(patient, false, now, 101)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        svc.for_patient(patient, true, now, 101)
            .await
            .unwrap()
            .len(),
        1
    );

    // Refusals: unknown scope, no scope, unknown basis, blank recorder, self, past or over-long expiry.
    let bad = scopes(&["delete"]);
    assert!(
        svc.grant(patient, &new(proxy, &bad, 10), now, &sys)
            .await
            .is_err()
    );
    assert!(
        svc.grant(patient, &new(proxy, &[], 10), now, &sys)
            .await
            .is_err()
    );
    let mut b = new(proxy, &all, 10);
    b.basis = "vibes";
    assert!(svc.grant(patient, &b, now, &sys).await.is_err());
    let mut b = new(proxy, &all, 10);
    b.recorded_by = " ";
    assert!(svc.grant(patient, &b, now, &sys).await.is_err());
    assert!(
        svc.grant(patient, &new(patient, &all, 10), now, &sys)
            .await
            .is_err()
    );
    assert!(
        svc.grant(patient, &new(proxy, &all, -1), now, &sys)
            .await
            .is_err()
    );
    assert!(
        svc.grant(patient, &new(proxy, &all, 4000), now, &sys)
            .await
            .is_err()
    );
}
