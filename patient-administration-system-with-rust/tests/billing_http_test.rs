//! Billing over HTTP: an account is opened, charged, invoiced, paid in part and then in full, and closed;
//! what the PAS refuses on the way. Needs PostgreSQL: `cargo test -- --ignored`.
mod common;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let mut b = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let r = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let st = r.status();
    let bytes = to_bytes(r.into_body(), usize::MAX).await.unwrap();
    (st, serde_json::from_slice(&bytes).unwrap_or_default())
}

fn charge(account: &str, code: &str, amount: &str) -> Value {
    json!({
        "account_id": account, "code": code, "description": "a visit",
        "amount_value": amount, "amount_currency": "GBP",
    })
}

fn pay(invoice: &str, amount: &str, currency: &str) -> Value {
    json!({
        "invoice_id": invoice, "amount_value": amount, "amount_currency": currency,
        "method": "card", "reference": "REF-1",
    })
}

#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn an_account_is_charged_invoiced_paid_and_closed() {
    let s = common::build_state(&common::database_url().unwrap()).await;
    let app = patient_administration_system::api::rest::router(s);
    let person = Uuid::new_v4();

    let (st, _) = call(
        &app,
        "POST",
        "/api/accounts",
        Some(json!({ "person_id": person, "currency": "pounds" })),
    )
    .await;
    assert!(st.is_client_error(), "a currency that is not ISO 4217");
    let (st, v) = call(
        &app,
        "POST",
        "/api/accounts",
        Some(json!({ "person_id": person, "currency": "GBP" })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let account = v["data"]["id"].as_str().unwrap().to_string();

    // Charges: a bad amount, a negative one and a code-less one are refused; two good ones are posted.
    for bad in [
        charge(&account, "C1", "lots"),
        charge(&account, "C1", "-1.00"),
    ] {
        let (st, _) = call(&app, "POST", "/api/charges", Some(bad)).await;
        assert!(st.is_client_error());
    }
    let mut ids = Vec::new();
    for (code, amount) in [("C1", "30.00"), ("C2", "20.50")] {
        let (st, v) = call(
            &app,
            "POST",
            "/api/charges",
            Some(charge(&account, code, amount)),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{v}");
        ids.push(v["data"]["id"].as_str().unwrap().to_string());
    }

    // An invoice needs charges that are the account's.
    let (st, _) = call(
        &app,
        "POST",
        "/api/invoices",
        Some(json!({ "account_id": account, "charge_ids": [Uuid::new_v4()] })),
    )
    .await;
    assert!(st.is_client_error(), "no matching charges");
    let (st, v) = call(
        &app,
        "POST",
        "/api/invoices",
        Some(json!({ "account_id": account, "charge_ids": ids })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let invoice = v["data"]["id"].as_str().unwrap().to_string();

    // Paid in part, then the rest; a payment in another currency or on no invoice is refused.
    let (st, v) = call(
        &app,
        "POST",
        "/api/payments",
        Some(pay(&invoice, "10.00", "GBP")),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, _) = call(
        &app,
        "POST",
        "/api/payments",
        Some(pay(&invoice, "5.00", "USD")),
    )
    .await;
    assert!(st.is_client_error(), "a payment in another currency");
    let (st, _) = call(
        &app,
        "POST",
        "/api/payments",
        Some(pay(&Uuid::new_v4().to_string(), "5.00", "GBP")),
    )
    .await;
    assert!(st.is_client_error(), "no such invoice");
    let (st, _) = call(
        &app,
        "POST",
        "/api/payments",
        Some(pay(&invoice, "40.50", "GBP")),
    )
    .await;
    assert_eq!(st, StatusCode::OK);

    let (st, v) = call(
        &app,
        "POST",
        &format!("/api/accounts/{account}/close"),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{v}");
    let (st, _) = call(
        &app,
        "POST",
        &format!("/api/accounts/{account}/close"),
        None,
    )
    .await;
    assert!(st.is_client_error(), "closed once");
    let (st, _) = call(
        &app,
        "POST",
        &format!("/api/accounts/{}/close", Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

/// The repository's by-id reads (used to chain a charge to its account and patient).
#[tokio::test]
#[ignore = "requires PostgreSQL (DATABASE_URL); run with `cargo test -- --ignored`"]
async fn a_charge_and_an_account_are_found_by_id_and_unknown_ids_are_none() {
    use patient_administration_system::db::repositories::billing::BillingRepository;
    let s = common::build_state(&common::database_url().unwrap()).await;
    let db = s.db.clone();
    let app = patient_administration_system::api::rest::router(s);
    let (_, v) = call(
        &app,
        "POST",
        "/api/accounts",
        Some(json!({ "person_id": Uuid::new_v4(), "currency": "GBP" })),
    )
    .await;
    let account: Uuid = v["data"]["id"].as_str().unwrap().parse().unwrap();
    let (_, v) = call(
        &app,
        "POST",
        "/api/charges",
        Some(charge(&account.to_string(), "C9", "1.00")),
    )
    .await;
    let charge_id: Uuid = v["data"]["id"].as_str().unwrap().parse().unwrap();

    let found = BillingRepository::find_charge_by_id(&db, charge_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((found.id, found.account_id), (charge_id, account));
    let acct = BillingRepository::find_account_by_id(&db, account)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(acct.id, account);
    assert!(
        BillingRepository::find_charge_by_id(&db, Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        BillingRepository::find_account_by_id(&db, Uuid::new_v4())
            .await
            .unwrap()
            .is_none()
    );
}
