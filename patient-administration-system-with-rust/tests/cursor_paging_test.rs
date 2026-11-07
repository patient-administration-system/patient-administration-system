//! NFR-23: a cut list gives its next page as a signed cursor, in `meta.next` and `Link: rel="next"`, and the
//! pages together are the whole list, once each, in order. A cursor works only on the list and filters it came
//! from. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::LINK},
};
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::referrals::NewReferral;
use tower::ServiceExt;
use uuid::Uuid;

async fn get(app: &axum::Router, uri: &str) -> (StatusCode, Option<String>, serde_json::Value) {
    let r = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = r.status();
    let link = r
        .headers()
        .get(LINK)
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (
        status,
        link,
        serde_json::from_slice(&bytes).unwrap_or_default(),
    )
}

fn ids(v: &serde_json::Value) -> Vec<String> {
    v["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_string())
        .collect()
}

async fn seeded(n: usize) -> (axum::Router, String) {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let service = format!("svc-{}", Uuid::new_v4());
    for _ in 0..n {
        s.referrals
            .create(
                NewReferral {
                    person_id: Uuid::new_v4(),
                    source: "gp".into(),
                    external_ref: None,
                    target_service: service.clone(),
                    reason: None,
                    referring_worker_id: None,
                    received_at: None,
                },
                &UserContext::system(),
            )
            .await
            .unwrap();
    }
    (patient_administration_system::api::rest::router(s), service)
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_pages_together_are_the_whole_list_once_each_in_order() {
    let (app, service) = seeded(5).await;
    let (st, _, whole) = get(&app, &format!("/api/referrals?service={service}&limit=500")).await;
    assert_eq!(st, StatusCode::OK, "{whole}");
    let whole = ids(&whole);
    assert_eq!(whole.len(), 5);

    let mut seen = Vec::new();
    let mut uri = format!("/api/referrals?service={service}&limit=2");
    let mut pages = 0;
    loop {
        pages += 1;
        let (st, link, v) = get(&app, &uri).await;
        assert_eq!(st, StatusCode::OK, "{uri}: {v}");
        seen.extend(ids(&v));
        let next = v["meta"]["next"].as_str().map(str::to_string);
        assert_eq!(v["meta"]["truncated"], next.is_some(), "{v}");
        match (next, link) {
            (Some(next), Some(link)) => {
                assert_eq!(link, format!("<{next}>; rel=\"next\""));
                assert!(next.contains(&format!("service={service}")), "{next}");
                uri = next;
            }
            (None, None) => break,
            other => panic!("meta.next and Link disagree: {other:?}"),
        }
        assert!(pages < 10, "the pages never end");
    }
    assert_eq!(pages, 3, "5 rows at 2 a page");
    assert_eq!(seen, whole, "each row once, in the list's order");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_cursor_works_only_on_its_own_list_and_filters() {
    let (app, service) = seeded(3).await;
    let (_, _, v) = get(&app, &format!("/api/referrals?service={service}&limit=1")).await;
    let next = v["meta"]["next"].as_str().unwrap().to_string();
    let cursor = next.split("cursor=").nth(1).unwrap().to_string();

    // A different limit is fine: the second row is where the next page starts.
    let (st, _, v) = get(
        &app,
        &format!("/api/referrals?service={service}&limit=5&cursor={cursor}"),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"].as_array().unwrap().len(), 2, "{v}");

    let mut tampered = cursor.clone();
    let last = tampered.pop().unwrap();
    tampered.push(if last == 'A' { 'B' } else { 'A' });
    for (name, uri) in [
        (
            "another filter",
            format!("/api/referrals?service=other&cursor={cursor}"),
        ),
        ("no filter", format!("/api/referrals?cursor={cursor}")),
        (
            "another list",
            format!("/api/support-flags/open?cursor={cursor}"),
        ),
        (
            "tampered",
            format!("/api/referrals?service={service}&cursor={tampered}"),
        ),
        (
            "made up",
            format!("/api/referrals?service={service}&cursor=made-up"),
        ),
    ] {
        let (st, link, v) = get(&app, &uri).await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{name}: {v}");
        assert_eq!(v["code"], "unprocessable_entity", "{name}: {v}");
        assert!(link.is_none(), "{name}");
        assert!(
            !v.to_string().contains(&cursor),
            "{name}: the refusal quotes nothing sent"
        );
    }
}
