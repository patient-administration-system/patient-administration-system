//! REST handlers for billing.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::Error;
use crate::api::ApiResponse;
use crate::api::rest::AppState;
use crate::api::rest::handlers::{HandlerResult, error_to_response};
use crate::api::rest::upstream_proxy::user_context_from_headers;
use crate::billing::PostChargeInput;
use crate::db::repositories::audit::UserContext as RepoUserContext;
use crate::models::billing::PaymentMethod;
use crate::models::{Iso4217, Money};

fn audit_ctx(uc: &crate::upstream::UserContext) -> RepoUserContext {
    RepoUserContext {
        user_id: uc.user_id.clone(),
        user_ip: uc.user_ip.clone(),
        user_agent: uc.user_agent.clone(),
    }
}

fn ok(value: Value) -> HandlerResult {
    (StatusCode::OK, Json(ApiResponse::ok(value)))
}

/// Body shape for money fields on the wire — keeps the amount as a
/// string-encoded decimal so JSON-number imprecision can never bite.
#[derive(Debug, Deserialize)]
pub struct MoneyBody {
    pub amount_value: String,
    pub amount_currency: String,
}

impl MoneyBody {
    fn into_money(self) -> Result<Money, Error> {
        let amount = self
            .amount_value
            .parse::<Decimal>()
            .map_err(|e| Error::validation(format!("amount_value not a decimal: {e}")))?;
        let currency = Iso4217::new(&self.amount_currency)?;
        Ok(Money::new(amount, currency))
    }
}

#[derive(Debug, Deserialize)]
pub struct OpenAccountBody {
    pub person_id: Uuid,
    pub currency: String,
}

#[derive(Debug, Deserialize)]
pub struct PostChargeBody {
    pub account_id: Uuid,
    pub code: String,
    pub description: String,
    #[serde(flatten)]
    pub money: MoneyBody,
    #[serde(default)]
    pub encounter_id: Option<Uuid>,
    #[serde(default)]
    pub appointment_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct FinalizeInvoiceBody {
    pub account_id: Uuid,
    pub charge_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct PostPaymentBody {
    pub invoice_id: Uuid,
    #[serde(flatten)]
    pub money: MoneyBody,
    pub method: PaymentMethod,
    #[serde(default)]
    pub reference: Option<String>,
}

/// `POST /api/accounts` — open a billing account.
#[utoipa::path(
    post,
    path = "/api/accounts",
    tag = "Accounts",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn open_account(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<OpenAccountBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let currency = match Iso4217::new(&body.currency) {
        Ok(c) => c,
        Err(e) => return error_to_response(e),
    };
    match state
        .billing
        .open_account(body.person_id, currency, &ctx)
        .await
    {
        Ok(a) => ok(serde_json::to_value(a).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/charges`.
#[utoipa::path(
    post,
    path = "/api/charges",
    tag = "Charges",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn post_charge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PostChargeBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let amount = match body.money.into_money() {
        Ok(m) => m,
        Err(e) => return error_to_response(e),
    };
    let input = PostChargeInput {
        account_id: body.account_id,
        code: body.code,
        description: body.description,
        amount,
        encounter_id: body.encounter_id,
        appointment_id: body.appointment_id,
    };
    match state.billing.post_charge(input, &ctx).await {
        Ok(c) => ok(serde_json::to_value(c).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/invoices` — finalize an invoice from the named charge ids.
#[utoipa::path(
    post,
    path = "/api/invoices",
    tag = "Invoices",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn finalize_invoice(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<FinalizeInvoiceBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state
        .billing
        .finalize_invoice(body.account_id, body.charge_ids, &ctx)
        .await
    {
        Ok(inv) => ok(serde_json::to_value(inv).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/payments`.
#[utoipa::path(
    post,
    path = "/api/payments",
    tag = "Payments",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn post_payment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PostPaymentBody>,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    let amount = match body.money.into_money() {
        Ok(m) => m,
        Err(e) => return error_to_response(e),
    };
    match state
        .billing
        .post_payment(body.invoice_id, amount, body.method, body.reference, &ctx)
        .await
    {
        Ok(p) => ok(serde_json::to_value(p).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}

/// `POST /api/accounts/:id/close`.
#[utoipa::path(
    post,
    path = "/api/accounts/{id}/close",
    tag = "Accounts",
    params(("id" = String, Path, description = "Path parameter `id`")),
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn close_account(
    State(state): State<AppState>,
    Path(account_id): Path<Uuid>,
    headers: HeaderMap,
) -> HandlerResult {
    let ctx = audit_ctx(&user_context_from_headers(&headers));
    match state.billing.close_account(account_id, &ctx).await {
        Ok(a) => ok(serde_json::to_value(a).unwrap_or(Value::Null)),
        Err(e) => error_to_response(e),
    }
}
