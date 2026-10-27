//! Event subscriptions (NEW-4): topic-based FHIR R5 Subscriptions for appointment and waiting-list changes,
//! delivered from the existing outbox with retries and signed webhooks.
//!
//! The outbox has one global publisher and marks a row done once, so each subscriber gets its own row in
//! `subscription_deliveries`, unique per subscriber and event. A pass of [`SubscriptionService::tick`] first
//! copies each new outbox event into a pending delivery for every active subscriber of its topic (events
//! from before the subscription began are not sent), then posts the due ones. A failure is retried after
//! `30 s x 2^(attempt - 1)`, at most an hour, until `PAS_OUTBOX_MAX_RETRIES` attempts (0 means no limit),
//! then the delivery is `failed` and stays for an administrator to see. All state is in the database, so a
//! restart loses nothing.
//!
//! What is sent: an R5 `subscription-notification` Bundle with a `SubscriptionStatus` and, for an
//! appointment event, the reference to the appointment (`id-only`: no patient data in the message). The
//! `X-PAS-Signature` header is `sha256=` plus the HMAC-SHA256 of the body, keyed by the environment variable
//! the subscription names (`PAS_SUB_SECRET_...`); a subscription naming a variable that is not set is not
//! sent to rather than sent unsigned. Redirects are not followed. The endpoint must be `https`, or `http` on
//! the machine itself.
//!
//! **Not built:** the handshake and heartbeat notifications, `query-event` and the R4 Backport (NEW-1),
//! `full-resource` content, filters, a referral topic (referrals publish no outbox event yet, QUAL-35), and
//! a block on endpoints that resolve to private addresses (the administrator who registers one is trusted).
//! An event the global publisher dead-lettered before this pass ran is not delivered.

use std::time::Duration;

use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::config::env;
use crate::db::repositories::audit::{AuditLogRepository, UserContext};
use crate::db::row::RowExt;
use crate::streaming::WebhookEventPublisher;
use crate::{Error, Result};

/// The topics, with the canonical URL each is known by in a notification.
pub const TOPICS: [(&str, &str); 2] = [
    (
        "appointment",
        "urn:patient-administration-system:subscription-topic:appointment",
    ),
    (
        "waiting-list",
        "urn:patient-administration-system:subscription-topic:waiting-list",
    ),
];

/// The topic an outbox event type belongs to, if any.
pub fn topic_of(event_type: &str) -> Option<&'static str> {
    if event_type.starts_with("Appointment") {
        Some("appointment")
    } else if event_type.starts_with("Waitlist") {
        Some("waiting-list")
    } else {
        None
    }
}

/// The SQL `LIKE` pattern that selects a topic's events.
fn pattern(topic: &str) -> &'static str {
    if topic == "appointment" {
        "Appointment%"
    } else {
        "Waitlist%"
    }
}

/// The canonical URL of a topic.
pub fn topic_url(topic: &str) -> Option<&'static str> {
    TOPICS.iter().find(|(t, _)| *t == topic).map(|(_, u)| *u)
}

/// Seconds to wait after the `attempts`-th failure.
pub fn backoff_seconds(attempts: i32) -> i64 {
    let exp = u32::try_from((attempts - 1).clamp(0, 12)).unwrap_or(12);
    (30_i64 << exp).min(3_600)
}

/// An endpoint a subscription may use: `https`, or `http` on the machine itself.
pub fn check_endpoint(endpoint: &str) -> Result<()> {
    let url = reqwest::Url::parse(endpoint)
        .map_err(|_| Error::validation("endpoint must be a full URL"))?;
    if endpoint.len() > 500 {
        return Err(Error::validation("endpoint must be at most 500 characters"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Error::validation(
            "endpoint must not carry a user name or password",
        ));
    }
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    match url.scheme() {
        "https" => Ok(()),
        "http" if local => Ok(()),
        _ => Err(Error::validation(
            "endpoint must be https (http is allowed only for localhost)",
        )),
    }
}

/// The name of the environment variable that holds a subscription's signing secret.
pub fn check_secret_env(name: &str) -> Result<()> {
    let ok = name.starts_with("PAS_SUB_SECRET_")
        && name.len() > "PAS_SUB_SECRET_".len()
        && name.len() <= 80
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if ok {
        Ok(())
    } else {
        Err(Error::validation(
            "secret_env must be an environment variable name like PAS_SUB_SECRET_PARTNER",
        ))
    }
}

crate::string_enum! {
    /// Whether a subscription still delivers.
    pub enum SubscriptionState {
        Active = "active",
        Off = "off",
    }
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct Subscription {
    pub id: Uuid,
    pub topic: String,
    pub endpoint: String,
    #[schema(required = true)]
    pub secret_env: Option<String>,
    pub status: SubscriptionState,
    pub created_by: String,
    pub created_at: DateTime<Utc>,
    #[schema(required = true)]
    pub ended_at: Option<DateTime<Utc>>,
    pub pending: i64,
    pub delivered: i64,
    pub failed: i64,
}

/// The notification Bundle for one event. `number` is how many events this subscription has had so far,
/// this one included.
pub fn notification(
    subscription: Uuid,
    topic: &str,
    event_type: &str,
    payload: &Value,
    event_at: DateTime<Utc>,
    number: i64,
) -> Value {
    let focus = payload["appointment_id"]
        .as_str()
        .filter(|id| Uuid::parse_str(id).is_ok())
        .map(|id| format!("Appointment/{id}"));
    let mut event = json!({ "eventNumber": number.to_string(), "timestamp": event_at });
    if let Some(f) = &focus {
        event["focus"] = json!({ "reference": f });
    }
    let status = json!({
        "resourceType": "SubscriptionStatus",
        "status": "active",
        "type": "event-notification",
        "eventsSinceSubscriptionStart": number.to_string(),
        "notificationEvent": [event],
        "subscription": { "reference": format!("Subscription/{subscription}") },
        "topic": topic_url(topic),
    });
    let mut entries =
        vec![json!({ "fullUrl": format!("urn:uuid:{}", Uuid::new_v4()), "resource": status })];
    if let Some(f) = focus {
        entries.push(json!({
            "fullUrl": f,
            "request": { "method": "PUT", "url": f },
            "response": { "status": "200" },
        }));
    }
    json!({
        "resourceType": "Bundle",
        "type": "subscription-notification",
        "timestamp": event_at,
        "meta": { "tag": [{ "code": event_type }] },
        "entry": entries,
    })
}

/// Deliveries taken per subscription in one pass, so one subscriber with a long backlog (or a dead endpoint)
/// cannot keep the others waiting.
const PER_SUBSCRIPTION: i64 = 16;

pub struct SubscriptionService {
    db: DatabaseConnection,
    client: reqwest::Client,
}

fn subscription_of(r: &sea_orm::QueryResult) -> Result<Subscription> {
    Ok(Subscription {
        id: r.col("id")?,
        topic: r.col("topic")?,
        endpoint: r.col("endpoint")?,
        secret_env: r.col("secret_env")?,
        status: r.col::<String>("status")?.parse()?,
        created_by: r.col("created_by")?,
        created_at: r.col("created_at")?,
        ended_at: r.col("ended_at")?,
        pending: r.col("pending")?,
        delivered: r.col("delivered")?,
        failed: r.col("failed")?,
    })
}

const LIST_SQL: &str = "SELECT s.id, s.topic, s.endpoint, s.secret_env, s.status, s.created_by, s.created_at, \
        s.ended_at, \
        (SELECT count(*) FROM subscription_deliveries d WHERE d.subscription_id = s.id AND d.status = 'pending') AS pending, \
        (SELECT count(*) FROM subscription_deliveries d WHERE d.subscription_id = s.id AND d.status = 'delivered') AS delivered, \
        (SELECT count(*) FROM subscription_deliveries d WHERE d.subscription_id = s.id AND d.status = 'failed') AS failed \
     FROM subscriptions s";

impl SubscriptionService {
    pub fn new(db: DatabaseConnection) -> Self {
        let timeout =
            Duration::from_secs(u64::try_from(env::int("PAS_WEBHOOK_TIMEOUT_SECS")).unwrap_or(10));
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();
        Self { db, client }
    }

    /// Register a subscriber. `created_by` is the administrator, by name.
    pub async fn create(
        &self,
        topic: &str,
        endpoint: &str,
        secret_env: Option<&str>,
        created_by: &str,
        ctx: &UserContext,
    ) -> Result<Subscription> {
        if topic_url(topic).is_none() {
            return Err(Error::validation(format!(
                "topic must be one of {}",
                TOPICS.map(|(t, _)| t).join(", ")
            )));
        }
        check_endpoint(endpoint)?;
        let secret_env = secret_env.map(str::trim).filter(|s| !s.is_empty());
        if let Some(s) = secret_env {
            check_secret_env(s)?;
        }
        let by = crate::validation::named_person(
            created_by,
            "the administrator registering the subscription must be named",
        )?;
        let id = Uuid::new_v4();
        let txn = self.db.begin().await.map_err(Error::Database)?;
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "INSERT INTO subscriptions (id, topic, endpoint, secret_env, created_by, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6)",
            // The application's clock, as for `outbox_events.at` that it is compared with.
            [
                id.into(),
                topic.into(),
                endpoint.into(),
                secret_env.into(),
                by.into(),
                Utc::now().into(),
            ],
        ))
        .await
        .map_err(Error::Database)?;
        AuditLogRepository::log(
            &txn,
            "subscription",
            id,
            "create",
            None,
            Some(json!({ "topic": topic, "endpoint": endpoint, "secret_env": secret_env, "created_by": by })),
            ctx,
        )
        .await?;
        txn.commit().await.map_err(Error::Database)?;
        self.get(id).await
    }

    pub async fn get(&self, id: Uuid) -> Result<Subscription> {
        let row = self
            .db
            .query_one_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                format!("{LIST_SQL} WHERE s.id = $1"),
                [id.into()],
            ))
            .await
            .map_err(Error::Database)?
            .ok_or_else(|| Error::not_found(format!("subscription {id}")))?;
        subscription_of(&row)
    }

    pub async fn list(&self) -> Result<Vec<Subscription>> {
        self.db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                format!("{LIST_SQL} ORDER BY s.created_at, s.id"),
            ))
            .await
            .map_err(Error::Database)?
            .iter()
            .map(subscription_of)
            .collect()
    }

    /// Switch a subscription off. Deliveries not yet made are dropped from the queue (marked failed with the
    /// reason); the history stays.
    pub async fn end(&self, id: Uuid, ctx: &UserContext) -> Result<Subscription> {
        let txn = self.db.begin().await.map_err(Error::Database)?;
        let done = txn
            .execute_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "UPDATE subscriptions SET status = 'off', ended_at = now() WHERE id = $1 AND status = 'active'",
                [id.into()],
            ))
            .await
            .map_err(Error::Database)?;
        if done.rows_affected() == 0 {
            return Err(Error::not_found(format!("active subscription {id}")));
        }
        txn.execute_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            "UPDATE subscription_deliveries SET status = 'failed', last_error = 'subscription ended' \
             WHERE subscription_id = $1 AND status = 'pending'",
            [id.into()],
        ))
        .await
        .map_err(Error::Database)?;
        AuditLogRepository::log(&txn, "subscription", id, "end", None, None, ctx).await?;
        txn.commit().await.map_err(Error::Database)?;
        self.get(id).await
    }

    /// One pass: queue new events for each active subscriber, then post the deliveries that are due.
    /// Returns how many were delivered.
    pub async fn tick(&self, now: DateTime<Utc>) -> Result<usize> {
        self.fan_out(now).await?;
        self.deliver_due(now).await
    }

    async fn fan_out(&self, now: DateTime<Utc>) -> Result<()> {
        for topic in TOPICS.map(|(t, _)| t) {
            self.db
                .execute_raw(Statement::from_sql_and_values(
                    DbBackend::Postgres,
                    "INSERT INTO subscription_deliveries \
                       (id, subscription_id, outbox_event_id, event_type, payload, event_at, next_attempt_at) \
                     SELECT gen_random_uuid(), s.id, e.id, e.event_type, e.payload, e.at, $2 \
                     FROM subscriptions s JOIN outbox_events e ON e.event_type LIKE $3 AND e.at >= s.created_at \
                     WHERE s.status = 'active' AND s.topic = $1 \
                     ON CONFLICT (subscription_id, outbox_event_id) DO NOTHING",
                    [topic.into(), now.into(), pattern(topic).into()],
                ))
                .await
                .map_err(Error::Database)?;
        }
        Ok(())
    }

    async fn deliver_due(&self, now: DateTime<Utc>) -> Result<usize> {
        let due = self
            .db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Postgres,
                "SELECT * FROM ( \
                   SELECT d.id, d.subscription_id, d.event_type, d.payload, d.event_at, d.attempts, \
                          s.topic, s.endpoint, s.secret_env, \
                          (SELECT count(*) FROM subscription_deliveries x WHERE x.subscription_id = d.subscription_id \
                             AND (x.event_at, x.id) <= (d.event_at, d.id)) AS number, \
                          row_number() OVER (PARTITION BY d.subscription_id ORDER BY d.event_at, d.id) AS rn \
                   FROM subscription_deliveries d JOIN subscriptions s ON s.id = d.subscription_id \
                   WHERE d.status = 'pending' AND d.next_attempt_at <= $1 AND s.status = 'active') due \
                 WHERE rn <= $2 ORDER BY event_at, id LIMIT 256",
                [now.into(), PER_SUBSCRIPTION.into()],
            ))
            .await
            .map_err(Error::Database)?;
        let max = i32::try_from(env::int("PAS_OUTBOX_MAX_RETRIES")).unwrap_or(10);
        let mut delivered = 0;
        for r in &due {
            let id: Uuid = r.col("id")?;
            let attempts: i32 = r.col("attempts")?;
            let body = notification(
                r.col("subscription_id")?,
                &r.col::<String>("topic")?,
                &r.col::<String>("event_type")?,
                &r.col::<Value>("payload")?,
                r.col("event_at")?,
                r.col("number")?,
            );
            let result = self
                .post(
                    &r.col::<String>("endpoint")?,
                    r.col::<Option<String>>("secret_env")?.as_deref(),
                    id,
                    &r.col::<String>("event_type")?,
                    &body,
                )
                .await;
            match result {
                Ok(()) => {
                    self.db
                        .execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE subscription_deliveries SET status = 'delivered', attempts = attempts + 1, \
                               delivered_at = $2, last_error = NULL WHERE id = $1",
                            [id.into(), now.into()],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    delivered += 1;
                }
                Err(why) => {
                    let n = attempts + 1;
                    let give_up = max > 0 && n >= max;
                    self.db
                        .execute_raw(Statement::from_sql_and_values(
                            DbBackend::Postgres,
                            "UPDATE subscription_deliveries SET attempts = $2, last_error = $3, status = $4, \
                               next_attempt_at = $5 WHERE id = $1",
                            [
                                id.into(),
                                n.into(),
                                why.chars().take(300).collect::<String>().into(),
                                (if give_up { "failed" } else { "pending" }).into(),
                                (now + chrono::Duration::seconds(backoff_seconds(n))).into(),
                            ],
                        ))
                        .await
                        .map_err(Error::Database)?;
                    tracing::warn!(delivery = %id, attempt = n, "subscription delivery failed: {why}");
                }
            }
        }
        Ok(delivered)
    }

    /// POST one notification. The error text never includes the secret or the body.
    async fn post(
        &self,
        endpoint: &str,
        secret_env: Option<&str>,
        id: Uuid,
        event_type: &str,
        body: &Value,
    ) -> std::result::Result<(), String> {
        check_endpoint(endpoint).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec(body)
            .map_err(|_| "could not encode the notification".to_string())?;
        let mut req = self
            .client
            .post(endpoint)
            .header("content-type", "application/fhir+json; fhirVersion=5.0")
            .header("x-pas-event-id", id.to_string())
            .header("x-pas-event-type", event_type);
        if let Some(name) = secret_env {
            let secret = env::text(name).ok_or_else(|| {
                format!("the signing secret {name} is not set; not sent unsigned")
            })?;
            req = req.header(
                "x-pas-signature",
                WebhookEventPublisher::sign(secret.as_bytes(), &bytes),
            );
        }
        let resp = req
            .body(bytes)
            .send()
            .await
            .map_err(|e| format!("could not reach the endpoint: {}", e.without_url()))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("the endpoint answered {}", resp.status()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn events_belong_to_a_topic_or_none() {
        assert_eq!(topic_of("AppointmentBooked"), Some("appointment"));
        assert_eq!(topic_of("AppointmentNoShow"), Some("appointment"));
        assert_eq!(topic_of("WaitlistAdded"), Some("waiting-list"));
        assert_eq!(topic_of("ChargePosted"), None);
        assert_eq!(topic_of("EncounterAdmitted"), None);
    }

    #[test]
    fn the_retry_delay_doubles_to_an_hour() {
        let got: Vec<_> = [1, 2, 3, 4, 5, 7, 8, 30].map(backoff_seconds).to_vec();
        assert_eq!(got, vec![30, 60, 120, 240, 480, 1920, 3600, 3600]);
        assert_eq!(backoff_seconds(0), 30);
    }

    #[test]
    fn an_endpoint_is_https_or_local_and_carries_no_password() {
        for ok in [
            "https://partner.example/hook",
            "http://localhost:9000/x",
            "http://127.0.0.1:1/x",
        ] {
            assert!(check_endpoint(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://partner.example/hook",
            "ftp://x",
            "nonsense",
            "https://user:pw@partner.example/",
            "file:///etc/passwd",
        ] {
            assert!(check_endpoint(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_secret_is_named_by_an_environment_variable_in_one_prefix() {
        assert!(check_secret_env("PAS_SUB_SECRET_PARTNER").is_ok());
        for bad in [
            "PAS_SUB_SECRET_",
            "API_TOKEN",
            "PAS_SUB_SECRET_x",
            "pas_sub_secret_a",
            "PAS_SUB_SECRET_A B",
        ] {
            assert!(check_secret_env(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_notification_carries_the_status_and_only_the_appointments_reference() {
        let sub = Uuid::new_v4();
        let appt = Uuid::new_v4();
        let at = Utc.with_ymd_and_hms(2026, 10, 23, 12, 0, 0).unwrap();
        let n = notification(
            sub,
            "appointment",
            "AppointmentBooked",
            &json!({"appointment_id": appt, "person_id": Uuid::new_v4()}),
            at,
            3,
        );
        assert_eq!(n["type"], "subscription-notification");
        let status = &n["entry"][0]["resource"];
        assert_eq!(status["resourceType"], "SubscriptionStatus");
        assert_eq!(status["type"], "event-notification");
        assert_eq!(status["eventsSinceSubscriptionStart"], "3");
        assert_eq!(
            status["subscription"]["reference"],
            format!("Subscription/{sub}")
        );
        assert_eq!(
            status["notificationEvent"][0]["focus"]["reference"],
            format!("Appointment/{appt}")
        );
        assert!(
            !n.to_string().contains("person_id"),
            "no patient data in the message"
        );
        // A waiting-list event has no resource to point at.
        let w = notification(
            sub,
            "waiting-list",
            "WaitlistAdded",
            &json!({"entry_id": "x"}),
            at,
            1,
        );
        assert!(w["entry"][0]["resource"]["notificationEvent"][0]["focus"].is_null());
        assert_eq!(w["entry"].as_array().unwrap().len(), 1);
    }
}
