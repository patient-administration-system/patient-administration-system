//! NEW-2: waiting-list validation campaigns. The safety rules are the point: nothing removes a patient without
//! the steps and a named person, and a removal can be undone. Needs PostgreSQL: `cargo test -- --ignored`.

mod common;

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use patient_administration_system::api::rest::AppState;
use patient_administration_system::db::repositories::audit::UserContext;
use patient_administration_system::list_validation::NewCampaign;
use patient_administration_system::models::waitlist::Priority;
use sea_orm::{ConnectionTrait, DbBackend, Statement};
use tower::ServiceExt;
use uuid::Uuid;

fn sys() -> UserContext {
    UserContext::system()
}

struct W {
    s: AppState,
    service: String,
}

async fn world() -> W {
    let s = common::build_state(&common::database_url().unwrap()).await;
    W {
        s,
        service: format!("val-{}", Uuid::new_v4()),
    }
}

impl W {
    /// A person on the waiting list for the service whose clock started `weeks` ago. Returns (person, entry, pathway).
    async fn waiting(&self, weeks: i64) -> (Uuid, Uuid, Uuid) {
        let person = Uuid::new_v4();
        let entry = self
            .s
            .waitlist
            .add(
                None,
                person,
                self.service.clone(),
                Priority::Routine,
                &sys(),
            )
            .await
            .unwrap();
        let pathway = self
            .s
            .rtt
            .start(person, self.service.clone(), &sys())
            .await
            .unwrap();
        let started = format!("now() - interval '{weeks} weeks'");
        for sql in [
            format!(
                "UPDATE rtt_clock_events SET event_at = {started} WHERE pathway_id = '{}' AND kind = 'started'",
                pathway.id
            ),
            format!(
                "UPDATE rtt_pathways SET started_at = {started} WHERE id = '{}'",
                pathway.id
            ),
        ] {
            self.s.db.execute_unprepared(&sql).await.unwrap();
        }
        (person, entry.id, pathway.id)
    }

    async fn campaign(&self, limit: i64) -> Uuid {
        self.s
            .list_validation
            .start(
                NewCampaign {
                    name: "test".into(),
                    started_by: "Ms Validator".into(),
                    service_type: Some(self.service.clone()),
                    min_weeks: Some(20),
                    reply_in_days: 14,
                    final_wait_days: 14,
                    limit,
                },
                &sys(),
            )
            .await
            .unwrap()
            .id
    }

    async fn only_item(&self, campaign: Uuid) -> Uuid {
        let items = self
            .s
            .list_validation
            .items(campaign, None, None, 10)
            .await
            .unwrap();
        assert_eq!(items.len(), 1, "{items:?}");
        items[0].id
    }

    async fn sql(&self, sql: &str) {
        self.s.db.execute_unprepared(sql).await.unwrap();
    }

    async fn scalar_text(&self, sql: &str) -> String {
        self.s
            .db
            .query_one_raw(Statement::from_string(DbBackend::Postgres, sql.to_string()))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "v")
            .unwrap()
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn the_cohort_is_the_long_waiters_of_the_service_and_no_one_else() {
    let w = world().await;
    w.waiting(30).await; // long enough
    w.waiting(2).await; // too recent
    let c = w.campaign(10).await;
    let items = w.s.list_validation.items(c, None, None, 10).await.unwrap();
    assert_eq!(items.len(), 1, "only the 30-week wait");
    assert_eq!(items[0].state, "selected");
    // A second campaign does not take a pathway that is already in one.
    let again =
        w.s.list_validation
            .start(
                NewCampaign {
                    name: "again".into(),
                    started_by: "Ms V".into(),
                    service_type: Some(w.service.clone()),
                    min_weeks: Some(20),
                    reply_in_days: 14,
                    final_wait_days: 14,
                    limit: 10,
                },
                &sys(),
            )
            .await;
    assert!(again.is_err(), "nobody left to ask");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_urgent_service_is_never_in_a_campaign_and_never_removed() {
    let w = world().await;
    w.waiting(40).await;
    // Listed for human follow-up before the campaign: nobody to ask.
    let urgent = world().await;
    urgent
        .s
        .follow_up
        .approve_policy(&urgent.service, "Dr A. Clinician", &sys())
        .await
        .unwrap();
    urgent.waiting(40).await;
    assert!(
        urgent
            .s
            .list_validation
            .start(
                NewCampaign {
                    name: "x".into(),
                    started_by: "Ms V".into(),
                    service_type: Some(urgent.service.clone()),
                    min_weeks: Some(20),
                    reply_in_days: 14,
                    final_wait_days: 14,
                    limit: 10,
                },
                &sys()
            )
            .await
            .is_err(),
        "an urgent service has no cohort"
    );
    // Listed after the campaign began: the decision still refuses.
    let c = w.campaign(10).await;
    let item = w.only_item(c).await;
    w.s.list_validation
        .record_answer(c, item, "no_longer_needed", "Ms V", &sys())
        .await
        .unwrap();
    w.s.follow_up
        .approve_policy(&w.service, "Dr A. Clinician", &sys())
        .await
        .unwrap();
    let e =
        w.s.list_validation
            .decide(c, item, "remove", "Ms V", None, &sys())
            .await
            .unwrap_err()
            .to_string();
    assert!(e.contains("urgent"), "{e}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn nobody_is_removed_without_the_steps_a_name_or_against_their_answer() {
    let w = world().await;
    let (person, entry, pathway) = w.waiting(30).await;
    let c = w.campaign(10).await;
    let item = w.only_item(c).await;
    let v = &w.s.list_validation;

    // Selected, asked, nothing else: no decision yet.
    assert!(
        v.decide(c, item, "remove", "Ms V", None, &sys())
            .await
            .is_err()
    );
    v.mark_asked(c, item, "Ms V", "phone", &sys())
        .await
        .unwrap();
    assert!(
        v.decide(c, item, "remove", "Ms V", None, &sys())
            .await
            .is_err()
    );
    // The letter is for after the reply date.
    assert!(v.letter_sent(c, item, "Ms V", &sys()).await.is_err());

    // Someone who still needs it is never removed; keeping them is allowed.
    v.record_answer(c, item, "still_needed", "Ms V", &sys())
        .await
        .unwrap();
    let e = v
        .decide(c, item, "remove", "Ms V", None, &sys())
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("still need"), "{e}");
    assert_eq!(
        w.scalar_text(&format!(
            "SELECT status AS v FROM waitlist_entries WHERE id = '{entry}'"
        ))
        .await,
        "waiting"
    );
    assert_eq!(
        w.scalar_text(&format!(
            "SELECT status AS v FROM rtt_pathways WHERE id = '{pathway}'"
        ))
        .await,
        "active"
    );
    let _ = person;

    // A second patient: "no longer needed" is not enough without a name.
    let w2 = world().await;
    let (_, entry2, pathway2) = w2.waiting(30).await;
    let c2 = w2.campaign(10).await;
    let item2 = w2.only_item(c2).await;
    let v2 = &w2.s.list_validation;
    v2.record_answer(c2, item2, "no_longer_needed", "Ms V", &sys())
        .await
        .unwrap();
    assert!(
        v2.decide(c2, item2, "remove", "  ", None, &sys())
            .await
            .is_err()
    );
    assert!(
        v2.decide(c2, item2, "maybe", "Ms V", None, &sys())
            .await
            .is_err()
    );
    assert_eq!(
        w2.scalar_text(&format!(
            "SELECT status AS v FROM waitlist_entries WHERE id = '{entry2}'"
        ))
        .await,
        "waiting",
        "a refused decision changes nothing"
    );

    // With a name, it is a removal: the entry is removed and the clock stopped for the right reason.
    let done = v2
        .decide(
            c2,
            item2,
            "remove",
            " Mr R. Manager ",
            Some("patient rang"),
            &sys(),
        )
        .await
        .unwrap();
    assert_eq!(done.state, "removed");
    assert_eq!(done.decided_by.as_deref(), Some("Mr R. Manager"));
    assert_eq!(
        w2.scalar_text(&format!(
            "SELECT status AS v FROM waitlist_entries WHERE id = '{entry2}'"
        ))
        .await,
        "removed"
    );
    assert_eq!(
        w2.scalar_text(&format!(
            "SELECT status AS v FROM rtt_pathways WHERE id = '{pathway2}'"
        ))
        .await,
        "stopped"
    );
    assert_eq!(
        w2.scalar_text(&format!(
            "SELECT reason AS v FROM rtt_clock_events WHERE pathway_id = '{pathway2}' AND kind = 'stopped'"
        ))
        .await,
        "validation_removal"
    );
    // Audited, and a second decision is refused.
    assert_eq!(
        w2.scalar_text(&format!(
            "SELECT count(*)::text AS v FROM audit_log WHERE entity_type = 'waitlist_entry' AND entity_id = '{entry2}' AND reason_code = 'validation_removal'"
        ))
        .await,
        "1"
    );
    assert!(
        v2.decide(c2, item2, "keep", "Ms V", None, &sys())
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_patient_who_never_replies_gets_a_letter_and_then_a_decision_after_the_wait() {
    let w = world().await;
    let (_, entry, _) = w.waiting(30).await;
    let c = w.campaign(10).await;
    let item = w.only_item(c).await;
    let v = &w.s.list_validation;
    v.mark_asked(c, item, "Ms V", "sms", &sys()).await.unwrap();

    // Before the reply date: no letter, nothing to decide.
    assert!(v.letter_sent(c, item, "Ms V", &sys()).await.is_err());
    let due = v.items(c, None, Some("letter_due"), 10).await.unwrap();
    assert!(due.is_empty());

    // The date passes with no reply: the letter is due, and a decision is still not.
    w.sql(&format!(
        "UPDATE validation_campaigns SET reply_by = current_date - 1 WHERE id = '{c}'"
    ))
    .await;
    assert_eq!(
        v.items(c, None, Some("letter_due"), 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        v.decide(c, item, "remove", "Ms V", None, &sys())
            .await
            .is_err(),
        "the letter comes first"
    );
    assert!(
        v.letter_sent(c, item, "  ", &sys()).await.is_err(),
        "a named person posts it"
    );
    let sent = v.letter_sent(c, item, "Ms V", &sys()).await.unwrap();
    assert_eq!(sent.state, "letter_sent");
    assert!(sent.final_by.is_some());

    // The wait for a reply to the letter has not ended: still no decision.
    let e = v
        .decide(c, item, "remove", "Mr R", None, &sys())
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("has not ended"), "{e}");

    // It ends with no reply: a person may decide (and nothing happened until they did).
    w.sql(&format!(
        "UPDATE validation_items SET final_by = current_date - 1 WHERE id = '{item}'"
    ))
    .await;
    assert_eq!(
        w.scalar_text(&format!(
            "SELECT status AS v FROM waitlist_entries WHERE id = '{entry}'"
        ))
        .await,
        "waiting"
    );
    assert_eq!(
        v.items(c, None, Some("decision_due"), 10)
            .await
            .unwrap()
            .len(),
        1
    );
    let kept = v
        .decide(c, item, "keep", "Mr R", Some("no contact details"), &sys())
        .await
        .unwrap();
    assert_eq!(kept.state, "kept");
    assert_eq!(
        w.scalar_text(&format!(
            "SELECT status AS v FROM waitlist_entries WHERE id = '{entry}'"
        ))
        .await,
        "waiting"
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_removal_can_be_undone_and_the_clock_carries_on_as_if_nothing_happened() {
    let w = world().await;
    let (_, entry, pathway) = w.waiting(30).await;
    let c = w.campaign(10).await;
    let item = w.only_item(c).await;
    let v = &w.s.list_validation;
    v.record_answer(c, item, "no_longer_needed", "Ms V", &sys())
        .await
        .unwrap();
    let before = w.s.rtt.weeks_waiting(pathway).await.unwrap();
    assert!(before >= 29, "{before}");
    v.decide(c, item, "remove", "Mr R", None, &sys())
        .await
        .unwrap();

    // Not removed items cannot be reinstated; removed ones can, with a name.
    assert!(v.reinstate(c, item, "  ", &sys()).await.is_err());
    // Time away does not count against the patient: pretend the removal was 10 days ago.
    w.sql(&format!(
        "UPDATE rtt_clock_events SET event_at = event_at - interval '10 days' WHERE pathway_id = '{pathway}' AND kind = 'stopped'"
    ))
    .await;
    w.sql(&format!(
        "UPDATE validation_items SET removed_at = removed_at - interval '10 days', rtt_stop_event_at = rtt_stop_event_at - interval '10 days' WHERE id = '{item}'"
    ))
    .await;
    let back = v.reinstate(c, item, "Mr R", &sys()).await.unwrap();
    assert_eq!(back.state, "reinstated");
    assert_eq!(
        w.scalar_text(&format!(
            "SELECT status AS v FROM waitlist_entries WHERE id = '{entry}'"
        ))
        .await,
        "waiting"
    );
    assert_eq!(
        w.scalar_text(&format!(
            "SELECT status AS v FROM rtt_pathways WHERE id = '{pathway}'"
        ))
        .await,
        "active"
    );
    let after = w.s.rtt.weeks_waiting(pathway).await.unwrap();
    assert!(
        after >= before,
        "the 10 days away are not taken off the wait: before {before}, after {after}"
    );
    assert!(v.reinstate(c, item, "Mr R", &sys()).await.is_err(), "once");

    // A removal older than the window is final.
    let w2 = world().await;
    w2.waiting(30).await;
    let c2 = w2.campaign(10).await;
    let item2 = w2.only_item(c2).await;
    let v2 = &w2.s.list_validation;
    v2.record_answer(c2, item2, "no_longer_needed", "Ms V", &sys())
        .await
        .unwrap();
    v2.decide(c2, item2, "remove", "Mr R", None, &sys())
        .await
        .unwrap();
    w2.sql(&format!(
        "UPDATE validation_items SET removed_at = now() - interval '100 days' WHERE id = '{item2}'"
    ))
    .await;
    let e = v2
        .reinstate(c2, item2, "Mr R", &sys())
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("90 days"), "{e}");
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn removals_are_reported_apart_from_treatment_and_the_http_surface_works() {
    let w = world().await;
    w.waiting(30).await;
    w.waiting(31).await;
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
            let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
            (
                st,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
            )
        }
    };
    let (st, v) = call(
        "POST",
        "/api/validation-campaigns".into(),
        Some(serde_json::json!({
            "name": "spring", "started_by": "Ms V", "service_type": w.service, "min_weeks": 20
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let campaign = v["data"]["id"].as_str().unwrap().to_string();
    assert_eq!(v["data"]["counts"]["selected"], 2);

    let (_, v) = call(
        "GET",
        format!("/api/validation-campaigns/{campaign}/items?limit=1"),
        None,
    )
    .await;
    assert_eq!(v["meta"]["truncated"], true);
    let item = v["data"][0]["id"].as_str().unwrap().to_string();

    let (st, v) = call(
        "POST",
        format!("/api/validation-campaigns/{campaign}/items/{item}/answer"),
        Some(serde_json::json!({ "answer": "no_longer_needed", "recorded_by": "Ms V" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, v) = call(
        "POST",
        format!("/api/validation-campaigns/{campaign}/items/{item}/decision"),
        Some(serde_json::json!({ "decision": "remove", "decided_by": "Mr R" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["state"], "removed");

    // The other patient has not been asked: a decision is a 409, not a removal.
    let (_, v) = call(
        "GET",
        format!("/api/validation-campaigns/{campaign}/items?state=selected"),
        None,
    )
    .await;
    let other = v["data"][0]["id"].as_str().unwrap().to_string();
    let (st, _) = call(
        "POST",
        format!("/api/validation-campaigns/{campaign}/items/{other}/decision"),
        Some(serde_json::json!({ "decision": "remove", "decided_by": "Mr R" })),
    )
    .await;
    assert_eq!(st, StatusCode::CONFLICT);

    let (st, v) = call("GET", "/api/reports/validation-removals".into(), None).await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let mine = v["data"]["by_service"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["service"] == w.service)
        .expect("this service in the report");
    assert_eq!(mine["removed"], 1);
    assert_eq!(mine["kept"], 0);
    assert!(
        !v.to_string().contains(&item),
        "the report names no one: no ids"
    );
}
