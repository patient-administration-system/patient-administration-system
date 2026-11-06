//! billing repository — Account, Charge, Invoice, Payment

use crate::db::sql::{Conn, Stmt, Table};
use std::str::FromStr;

use rust_decimal::Decimal;
use uuid::Uuid;

use crate::db::entities::{account, charge, invoice, payment};
use crate::models::billing::{
    Account, AccountStatus, Charge, Invoice, InvoiceStatus, Payment, PaymentMethod,
};
use crate::models::{Iso4217, Money};
use crate::{Error, Result};

pub struct BillingRepository;

impl BillingRepository {
    pub async fn create_account<C: Conn>(conn: &C, a: &Account) -> Result<Account> {
        let am = account_to_row(a);
        am.insert(conn).await?;
        Ok(a.clone())
    }

    pub async fn find_open_account_for_patient<C: Conn>(
        conn: &C,
        person_id: Uuid,
    ) -> Result<Option<Account>> {
        let m = conn
            .one_as::<account::Model>(Stmt::new(
                "SELECT * FROM accounts WHERE person_id = $1 AND status = $2 LIMIT 1",
                [
                    person_id.into(),
                    account_status_to_str(AccountStatus::Open).into(),
                ],
            ))
            .await?;
        m.map(account_from_model).transpose()
    }

    /// Close an account. The row is locked and an account already closed is refused with `409`, so a repeat
    /// cannot move `closed_at` or publish a second `AccountClosed` event (OFH-11).
    pub async fn close_account<C: Conn>(conn: &C, id: Uuid) -> Result<Account> {
        let m = conn
            .one_as::<account::Model>(Stmt::new(
                "SELECT * FROM accounts WHERE id = $1 LIMIT 1 FOR UPDATE",
                [id.into()],
            ))
            .await?
            .ok_or_else(|| Error::not_found(format!("account {id}")))?;
        if m.status == account_status_to_str(AccountStatus::Closed) {
            return Err(Error::conflict("that account is already closed"));
        }
        let now = chrono::Utc::now().fixed_offset();
        let mut am = m.patch();
        am.set(
            "status",
            account_status_to_str(AccountStatus::Closed).to_string(),
        );
        am.set("closed_at", Some(now));
        am.set("updated_at", now);
        let updated = am.update(conn).await?;
        account_from_model(updated)
    }

    pub async fn create_charge<C: Conn>(conn: &C, c: &Charge) -> Result<Charge> {
        let am = charge_to_row(c);
        am.insert(conn).await?;
        Ok(c.clone())
    }

    /// Locate a charge by its primary id. Used by the v0.19 HL7 v2
    /// outbound publisher (`ChargePosted → DFT^P03`) to chain
    /// charge → account → patient when the outbox payload only
    /// carries `charge_id`.
    pub async fn find_charge_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Charge>> {
        let m = conn
            .one_as::<charge::Model>(Stmt::new(
                "SELECT * FROM charges WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(charge_from_model).transpose()
    }

    /// Locate an account by its primary id. Companion to
    /// `find_charge_by_id`.
    pub async fn find_account_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Account>> {
        let m = conn
            .one_as::<account::Model>(Stmt::new(
                "SELECT * FROM accounts WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(account_from_model).transpose()
    }

    pub async fn list_charges_for_account<C: Conn>(
        conn: &C,
        account_id: Uuid,
    ) -> Result<Vec<Charge>> {
        let rows = conn
            .all_as::<charge::Model>(Stmt::new(
                "SELECT * FROM charges WHERE account_id = $1",
                [account_id.into()],
            ))
            .await?;
        rows.into_iter().map(charge_from_model).collect()
    }

    pub async fn create_invoice<C: Conn>(conn: &C, i: &Invoice) -> Result<Invoice> {
        let am = invoice_to_row(i)?;
        am.insert(conn).await?;
        Ok(i.clone())
    }

    pub async fn find_invoice_by_id<C: Conn>(conn: &C, id: Uuid) -> Result<Option<Invoice>> {
        let m = conn
            .one_as::<invoice::Model>(Stmt::new(
                "SELECT * FROM invoices WHERE id = $1 LIMIT 1",
                [id.into()],
            ))
            .await?;
        m.map(invoice_from_model).transpose()
    }

    pub async fn update_invoice<C: Conn>(conn: &C, i: &Invoice) -> Result<Invoice> {
        invoice_to_row(i)?.update_row(conn).await?;
        Ok(i.clone())
    }

    pub async fn create_payment<C: Conn>(conn: &C, p: &Payment) -> Result<Payment> {
        let am = payment_to_row(p);
        am.insert(conn).await?;
        Ok(p.clone())
    }

    pub async fn list_payments_for_invoice<C: Conn>(
        conn: &C,
        invoice_id: Uuid,
    ) -> Result<Vec<Payment>> {
        let rows = conn
            .all_as::<payment::Model>(Stmt::new(
                "SELECT * FROM payments WHERE invoice_id = $1",
                [invoice_id.into()],
            ))
            .await?;
        rows.into_iter().map(payment_from_model).collect()
    }
}

// --- enum conversions ---

pub(crate) fn account_status_to_str(s: AccountStatus) -> &'static str {
    match s {
        AccountStatus::Open => "open",
        AccountStatus::Closed => "closed",
    }
}

pub(crate) fn account_status_from_str(s: &str) -> Result<AccountStatus> {
    match s {
        "open" => Ok(AccountStatus::Open),
        "closed" => Ok(AccountStatus::Closed),
        other => Err(Error::internal(format!("unknown account status: {other}"))),
    }
}

pub(crate) fn invoice_status_to_str(s: InvoiceStatus) -> &'static str {
    match s {
        InvoiceStatus::Draft => "draft",
        InvoiceStatus::Finalized => "finalized",
        InvoiceStatus::Paid => "paid",
        InvoiceStatus::PartiallyPaid => "partially_paid",
        InvoiceStatus::Void => "void",
    }
}

pub(crate) fn invoice_status_from_str(s: &str) -> Result<InvoiceStatus> {
    match s {
        "draft" => Ok(InvoiceStatus::Draft),
        "finalized" => Ok(InvoiceStatus::Finalized),
        "paid" => Ok(InvoiceStatus::Paid),
        "partially_paid" => Ok(InvoiceStatus::PartiallyPaid),
        "void" => Ok(InvoiceStatus::Void),
        other => Err(Error::internal(format!("unknown invoice status: {other}"))),
    }
}

pub(crate) fn payment_method_to_str(m: PaymentMethod) -> &'static str {
    match m {
        PaymentMethod::Cash => "cash",
        PaymentMethod::Card => "card",
        PaymentMethod::BankTransfer => "bank_transfer",
        PaymentMethod::Insurance => "insurance",
        PaymentMethod::Other => "other",
    }
}

pub(crate) fn payment_method_from_str(s: &str) -> Result<PaymentMethod> {
    match s {
        "cash" => Ok(PaymentMethod::Cash),
        "card" => Ok(PaymentMethod::Card),
        "bank_transfer" => Ok(PaymentMethod::BankTransfer),
        "insurance" => Ok(PaymentMethod::Insurance),
        "other" => Ok(PaymentMethod::Other),
        other => Err(Error::internal(format!("unknown payment method: {other}"))),
    }
}

// --- struct conversions ---

fn account_to_row(a: &Account) -> account::Model {
    account::Model {
        id: a.id,
        person_id: a.person_id,
        status: account_status_to_str(a.status).to_string(),
        currency: a.currency.0.clone(),
        opened_at: a.opened_at.fixed_offset(),
        closed_at: a.closed_at.map(|t| t.fixed_offset()),
        created_at: a.created_at.fixed_offset(),
        updated_at: a.updated_at.fixed_offset(),
    }
}

fn account_from_model(m: account::Model) -> Result<Account> {
    Ok(Account {
        id: m.id,
        person_id: m.person_id,
        status: account_status_from_str(&m.status)?,
        currency: Iso4217::new(&m.currency)?,
        opened_at: m.opened_at.with_timezone(&chrono::Utc),
        closed_at: m.closed_at.map(|t| t.with_timezone(&chrono::Utc)),
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

fn charge_to_row(c: &Charge) -> charge::Model {
    charge::Model {
        id: c.id,
        account_id: c.account_id,
        encounter_id: c.encounter_id,
        appointment_id: c.appointment_id,
        code: c.code.clone(),
        description: c.description.clone(),
        amount_value: c.amount.amount.to_string(),
        amount_currency: c.amount.currency.0.clone(),
        posted_at: c.posted_at.fixed_offset(),
        created_at: c.created_at.fixed_offset(),
    }
}

fn charge_from_model(m: charge::Model) -> Result<Charge> {
    let amount_decimal = Decimal::from_str(&m.amount_value)
        .map_err(|e| Error::internal(format!("parse charge amount: {e}")))?;
    let currency = Iso4217::new(&m.amount_currency)?;
    Ok(Charge {
        id: m.id,
        account_id: m.account_id,
        encounter_id: m.encounter_id,
        appointment_id: m.appointment_id,
        code: m.code,
        description: m.description,
        amount: Money::new(amount_decimal, currency),
        posted_at: m.posted_at.with_timezone(&chrono::Utc),
        created_at: m.created_at.with_timezone(&chrono::Utc),
    })
}

fn invoice_to_row(i: &Invoice) -> Result<invoice::Model> {
    let charge_ids = serde_json::to_value(&i.charge_ids)
        .map_err(|e| Error::internal(format!("serialize charge_ids: {e}")))?;
    Ok(invoice::Model {
        id: i.id,
        account_id: i.account_id,
        status: invoice_status_to_str(i.status).to_string(),
        total_value: i.total.amount.to_string(),
        total_currency: i.total.currency.0.clone(),
        charge_ids,
        finalized_at: i.finalized_at.map(|t| t.fixed_offset()),
        created_at: i.created_at.fixed_offset(),
        updated_at: i.updated_at.fixed_offset(),
    })
}

fn invoice_from_model(m: invoice::Model) -> Result<Invoice> {
    let amount_decimal = Decimal::from_str(&m.total_value)
        .map_err(|e| Error::internal(format!("parse invoice total: {e}")))?;
    let currency = Iso4217::new(&m.total_currency)?;
    let charge_ids: Vec<Uuid> = serde_json::from_value(m.charge_ids)
        .map_err(|e| Error::internal(format!("deserialize charge_ids: {e}")))?;
    Ok(Invoice {
        id: m.id,
        account_id: m.account_id,
        status: invoice_status_from_str(&m.status)?,
        total: Money::new(amount_decimal, currency),
        charge_ids,
        finalized_at: m.finalized_at.map(|t| t.with_timezone(&chrono::Utc)),
        created_at: m.created_at.with_timezone(&chrono::Utc),
        updated_at: m.updated_at.with_timezone(&chrono::Utc),
    })
}

fn payment_to_row(p: &Payment) -> payment::Model {
    payment::Model {
        id: p.id,
        invoice_id: p.invoice_id,
        amount_value: p.amount.amount.to_string(),
        amount_currency: p.amount.currency.0.clone(),
        method: payment_method_to_str(p.method).to_string(),
        reference: p.reference.clone(),
        posted_at: p.posted_at.fixed_offset(),
        created_at: p.created_at.fixed_offset(),
    }
}

fn payment_from_model(m: payment::Model) -> Result<Payment> {
    let amount_decimal = Decimal::from_str(&m.amount_value)
        .map_err(|e| Error::internal(format!("parse payment amount: {e}")))?;
    let currency = Iso4217::new(&m.amount_currency)?;
    Ok(Payment {
        id: m.id,
        invoice_id: m.invoice_id,
        amount: Money::new(amount_decimal, currency),
        method: payment_method_from_str(&m.method)?,
        reference: m.reference,
        posted_at: m.posted_at.with_timezone(&chrono::Utc),
        created_at: m.created_at.with_timezone(&chrono::Utc),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn usd() -> Iso4217 {
        Iso4217::new("USD").unwrap()
    }

    #[test]
    fn test_invoice_roundtrip_via_row() {
        let mut inv = Invoice::new(Uuid::new_v4(), usd());
        inv.total = Money::new(Decimal::new(12345, 2), usd());
        inv.charge_ids = vec![Uuid::new_v4(), Uuid::new_v4()];
        let am = invoice_to_row(&inv).expect("to_row");
        let m = invoice::Model {
            id: am.id,
            account_id: am.account_id,
            status: am.status.clone(),
            total_value: am.total_value.clone(),
            total_currency: am.total_currency.clone(),
            charge_ids: am.charge_ids.clone(),
            finalized_at: am.finalized_at,
            created_at: am.created_at,
            updated_at: am.updated_at,
        };
        let back = invoice_from_model(m).expect("from_model");
        assert_eq!(back.id, inv.id);
        assert_eq!(back.status, InvoiceStatus::Draft);
        assert_eq!(back.total.amount, inv.total.amount);
        assert_eq!(back.total.currency, usd());
        assert_eq!(back.charge_ids.len(), 2);
    }
}
