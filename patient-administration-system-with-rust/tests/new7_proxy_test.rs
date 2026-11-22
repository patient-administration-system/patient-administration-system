//! NEW-7: carer and proxy authority. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use jiff::Timestamp;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::db::row::RowExt;
use patient_administration_system::db::sql::{Conn, Stmt};
use patient_administration_system::models::appointment::CancelledBy;
use patient_administration_system::proxy::{NewAuthority, ProxyService, proxy_context};
use patient_administration_system::scheduling::CancelDetails;
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
        expires_at: Timestamp::now() + patient_administration_system::time::days(days),
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
    let now = Timestamp::now();
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
        .one(Stmt::new("SELECT user_id FROM audit_log WHERE entity_id = $1 AND action ILIKE '%cancel%' ORDER BY at DESC LIMIT 1",
            [appt.id.into()],
        ))
        .await
        .unwrap()
        .expect("the cancellation was audited")
        .col("user_id")
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
    let now = Timestamp::now();
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

/// The same authority over HTTP: grant, list, the proxy reading, cancelling and rebooking, and what is refused.
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_http_surface_grants_lists_acts_and_revokes() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    let state = common::build_state(&common::database_url().unwrap()).await;
    let sys = UserContext::system();
    let schedule = common::new_schedule(&state.db).await.id;
    let (patient, proxy, stranger) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let appt = state
        .scheduling
        .book_slot(
            common::free_slot(&state.db, schedule, 5).await.id,
            patient,
            &sys,
        )
        .await
        .unwrap();
    let other_slot = common::free_slot(&state.db, schedule, 6).await.id;
    let app = patient_administration_system::api::rest::router(state);
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
            let bytes = to_bytes(r.into_body(), 1 << 20).await.unwrap();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
            )
        }
    };
    let grant = |who: Uuid, scopes: &[&str]| {
        serde_json::json!({
            "proxy_person_id": who,
            "relationship": "daughter",
            "basis": "patient_consent",
            "scopes": scopes,
            "expires_at": Timestamp::now() + patient_administration_system::time::days(30),
            "recorded_by": "Receptionist",
        })
    };

    // Grants: a bad scope is refused, a good one is recorded and listed.
    let (st, _) = call(
        "POST",
        format!("/api/patients/{patient}/proxies"),
        Some(grant(proxy, &["teleport"])),
    )
    .await;
    assert!(st.is_client_error(), "an unknown scope");
    let (st, v) = call(
        "POST",
        format!("/api/patients/{patient}/proxies"),
        Some(grant(proxy, &["receive", "cancel", "rebook"])),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let authority = v["data"]["id"].as_str().unwrap().to_string();
    let (st, v) = call("GET", format!("/api/patients/{patient}/proxies"), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v.to_string().contains(&authority));
    let (st, _) = call(
        "GET",
        format!("/api/patients/{patient}/proxies?limit=lots"),
        None,
    )
    .await;
    assert!(st.is_client_error());
    let (st, _) = call(
        "GET",
        format!("/api/patients/{patient}/proxies?include_inactive=true"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    // Reading: the proxy may, a stranger may not.
    let (st, v) = call(
        "GET",
        format!("/api/proxy/patients/{patient}/appointments?proxy_person_id={proxy}"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v.to_string().contains(&appt.id.to_string()));
    let (st, _) = call(
        "GET",
        format!("/api/proxy/patients/{patient}/appointments?proxy_person_id={stranger}"),
        None,
    )
    .await;
    assert!(st.is_client_error());

    // Rebooking needs a slot, a known appointment and the scope; then cancelling does too.
    let (st, _) = call(
        "POST",
        format!("/api/proxy/appointments/{}/rebook", appt.id),
        Some(serde_json::json!({ "proxy_person_id": proxy })),
    )
    .await;
    assert!(st.is_client_error(), "no slot_id");
    let (st, _) = call(
        "POST",
        format!("/api/proxy/appointments/{}/rebook", Uuid::new_v4()),
        Some(serde_json::json!({ "proxy_person_id": proxy, "slot_id": other_slot })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, _) = call(
        "POST",
        format!("/api/proxy/appointments/{}/rebook", appt.id),
        Some(serde_json::json!({ "proxy_person_id": stranger, "slot_id": other_slot })),
    )
    .await;
    assert!(st.is_client_error(), "a stranger cannot rebook");
    let (st, v) = call(
        "POST",
        format!("/api/proxy/appointments/{}/rebook", appt.id),
        Some(serde_json::json!({ "proxy_person_id": proxy, "slot_id": other_slot })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let moved = v["data"]["id"].as_str().unwrap().to_string();
    let (st, _) = call(
        "POST",
        format!("/api/proxy/appointments/{moved}/cancel"),
        Some(serde_json::json!({ "proxy_person_id": stranger })),
    )
    .await;
    assert!(st.is_client_error(), "a stranger cannot cancel");
    let (st, _) = call(
        "POST",
        format!("/api/proxy/appointments/{}/cancel", Uuid::new_v4()),
        Some(serde_json::json!({ "proxy_person_id": proxy })),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let (st, v) = call(
        "POST",
        format!("/api/proxy/appointments/{moved}/cancel"),
        Some(serde_json::json!({ "proxy_person_id": proxy })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");

    // Revoking: a named person, once; after it the proxy can do nothing.
    let (st, _) = call(
        "POST",
        format!("/api/proxies/{authority}/revoke"),
        Some(serde_json::json!({ "revoked_by": " " })),
    )
    .await;
    assert!(st.is_client_error(), "a blank name");
    let (st, v) = call(
        "POST",
        format!("/api/proxies/{authority}/revoke"),
        Some(serde_json::json!({ "revoked_by": "Receptionist" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["status"], "revoked");
    let (st, _) = call(
        "POST",
        format!("/api/proxies/{authority}/revoke"),
        Some(serde_json::json!({ "revoked_by": "Receptionist" })),
    )
    .await;
    assert!(st.is_client_error(), "only once");
    let (st, _) = call(
        "GET",
        format!("/api/proxy/patients/{patient}/appointments?proxy_person_id={proxy}"),
        None,
    )
    .await;
    assert!(st.is_client_error(), "a revoked proxy has nothing");
}
