//! Part of [`super`]: billing.

use super::*;

#[utoipa::path(
    post,
    path = "/api/hl7/v2/dft",
    tag = "Hl7",
    summary = "Dft",
    responses(
        (status = 200, description = "Success; the `ApiResponse` envelope"),
        (status = "4XX", description = "Client error; the `ApiResponse` envelope with a machine-readable code"),
        (status = "5XX", description = "Server or upstream error; the `ApiResponse` envelope"),
    )
)]
pub async fn dft(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let r = match resolve_person_or_ack(&state, &headers, &body).await {
        Ok(r) => r,
        Err(ack_resp) => return ack_resp,
    };
    let ft1_segs: Vec<_> = r.message.segments_named("FT1").collect();
    if ft1_segs.is_empty() {
        return ae(&r.sender, &r.control_id, "no FT1 segments");
    }
    let mut parsed: Vec<crate::hl7v2::pid_translation::Ft1> = Vec::with_capacity(ft1_segs.len());
    let mut shared_currency: Option<String> = None;
    for seg in &ft1_segs {
        let ft1 = match parse_ft1(seg) {
            Ok(f) => f,
            Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
        };
        if !matches!(ft1.transaction_type.as_str(), "CG" | "") {
            return ae(
                &r.sender,
                &r.control_id,
                &format!(
                    "FT1[{}]-6 transaction type {:?} not supported (CG only)",
                    ft1.set_id, ft1.transaction_type
                ),
            );
        }
        match &shared_currency {
            None => shared_currency = Some(ft1.currency.clone()),
            Some(c) if c == &ft1.currency => {}
            Some(c) => {
                return ae(
                    &r.sender,
                    &r.control_id,
                    &format!(
                        "FT1[{}] currency {:?} differs from prior {:?}",
                        ft1.set_id, ft1.currency, c
                    ),
                );
            }
        }
        parsed.push(ft1);
    }
    let Some(currency_code) = shared_currency else {
        return ae(&r.sender, &r.control_id, "no FT1 segments");
    };
    let currency = match Iso4217::new(&currency_code) {
        Ok(c) => c,
        Err(e) => return ae(&r.sender, &r.control_id, &client_text(&e)),
    };

    let person_id = r.person_id;
    let audit_clone = r.audit.clone();
    let parsed_clone = parsed.clone();
    let count = parsed.len();
    let outcome = state
        .db
        .transaction::<_, (uuid::Uuid, usize), crate::Error>(|txn| {
            Box::pin(async move {
                use crate::db::repositories::billing::BillingRepository;
                let account_id =
                    match BillingRepository::find_open_account_for_patient(txn, person_id).await? {
                        Some(a) => a.id,
                        None => {
                            let acct = Account::new(person_id, currency.clone());
                            let saved = BillingRepository::create_account(txn, &acct).await?;
                            AuditLogRepository::log(
                                txn,
                                "account",
                                saved.id,
                                "open_via_hl7v2_p03",
                                None,
                                Some(serde_json::to_value(&saved).unwrap_or_default()),
                                &audit_clone,
                            )
                            .await?;
                            saved.id
                        }
                    };
                for ft1 in &parsed_clone {
                    let amount = ft1.amount.parse::<rust_decimal::Decimal>().map_err(|e| {
                        crate::Error::validation(format!("FT1[{}]-11.1 amount: {e}", ft1.set_id))
                    })?;
                    let money = Money::new(amount, currency.clone());
                    let mut charge =
                        Charge::new(account_id, ft1.code.clone(), ft1.description.clone(), money);
                    if let Some(at) = ft1.posted_at {
                        charge.posted_at = at;
                    }
                    let saved = BillingRepository::create_charge(txn, &charge).await?;
                    AuditLogRepository::log(
                        txn,
                        "charge",
                        saved.id,
                        "post_via_hl7v2_p03",
                        None,
                        Some(serde_json::to_value(&saved).unwrap_or_default()),
                        &audit_clone,
                    )
                    .await?;
                    OutboxRepository::publish(
                        txn,
                        "ChargePosted",
                        &serde_json::json!({
                            "charge_id": saved.id,
                            "account_id": account_id,
                            "person_id": person_id,
                            "source": "hl7v2_p03",
                        }),
                    )
                    .await?;
                }
                Ok((account_id, count))
            })
        })
        .await;
    match outcome {
        Ok((account_id, n)) => {
            let diag = if n == 1 {
                format!("charge posted account={account_id}")
            } else {
                format!("charges_posted={n} account={account_id}")
            };
            aa(&r.sender, &r.control_id, Some(&diag))
        }
        Err(e) => ae(
            &r.sender,
            &r.control_id,
            &crate::Error::from(e).client_message(),
        ),
    }
}
