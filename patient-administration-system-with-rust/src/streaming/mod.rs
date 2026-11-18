//! streaming
//!
//! Domain event publishing for the Patient Administration System.
//!
//! [`DomainEvent`] is a typed envelope for outbox events. [`EventPublisher`]
//! is the async trait that backends implement. v0.1 ships two implementations:
//!
//! - [`InMemoryEventPublisher`] — stores events in a `Mutex<Vec<…>>` so tests
//!   and the in-process dispatcher can replay them.
//! - [`FluvioEventPublisher`] — stub that always returns `Error::Streaming`.
//!   Real Fluvio wiring is deferred.
//!
//! The [`dispatcher`] module polls the transactional outbox and forwards
//! events to the configured [`EventPublisher`].

pub mod dispatcher;
pub mod hl7v2_publisher;
pub mod webhook;

pub use hl7v2_publisher::Hl7v2MllpPublisher;
pub use webhook::WebhookEventPublisher;
// `CompositePublisher` is defined in this module — no re-export needed,
// but list it here in the module-level rustdoc so callers don't miss it.
//
// Use [`CompositePublisher`] when more than one backend is configured.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use uuid::Uuid;

use crate::Result;

/// A serialized domain event ready for publication.
///
/// `event_type` is a free-form string matching the names listed in `spec.md`
/// §6.3 (e.g. `"PatientCreated"`, `"EncounterAdmitted"`,
/// `"EncounterDischargeCancelled"`). `payload` is an already-encoded JSON
/// value so producers don't need to share types with consumers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainEvent {
    /// Unique event identifier (matches the outbox row id when applicable).
    pub id: Uuid,
    /// Event type tag, e.g. `"PatientCreated"`.
    pub event_type: String,
    /// JSON-encoded event body.
    pub payload: Value,
    /// Wall-clock time at which the event was constructed.
    pub at: jiff::Timestamp,
}

impl DomainEvent {
    /// Construct a new event with a fresh UUID and the current timestamp.
    pub fn new(event_type: impl Into<String>, payload: Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            event_type: event_type.into(),
            payload,
            at: jiff::Timestamp::now(),
        }
    }
}

/// Asynchronous publisher of [`DomainEvent`]s.
///
/// Implementations should be cheap to clone (or shared behind `Arc`) so they
/// can live inside `AppState`.
#[async_trait]
pub trait EventPublisher: Send + Sync {
    /// Publish a single event. Backends are expected to either persist the
    /// event durably or hand it to a transport such as Fluvio.
    async fn publish(&self, event: DomainEvent) -> Result<()>;
}

/// In-memory publisher used by tests and the outbox dispatcher.
///
/// Events are appended to an internal `Vec` guarded by a `tokio::sync::Mutex`
/// and can be inspected via [`InMemoryEventPublisher::events`].
pub struct InMemoryEventPublisher {
    events: tokio::sync::Mutex<Vec<DomainEvent>>,
}

impl InMemoryEventPublisher {
    /// Construct an empty in-memory publisher.
    pub fn new() -> Self {
        Self {
            events: tokio::sync::Mutex::new(Vec::new()),
        }
    }

    /// Return a snapshot of every event published so far.
    pub async fn events(&self) -> Vec<DomainEvent> {
        self.events.lock().await.clone()
    }
}

impl Default for InMemoryEventPublisher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventPublisher for InMemoryEventPublisher {
    async fn publish(&self, event: DomainEvent) -> Result<()> {
        self.events.lock().await.push(event);
        Ok(())
    }
}

/// Fan-out publisher (v0.14). Wraps a non-empty list of downstream
/// publishers and forwards every event to **all** of them in order.
///
/// Failure semantics are deliberately conservative: if **any** downstream
/// publisher returns an error, the composite returns that error. The
/// outbox dispatcher will then leave the row unpublished and retry it on
/// the next tick — which means receivers that already accepted the event
/// will see it again. Receivers must therefore be **idempotent on
/// `event.id`**, which is documented for the webhook publisher's
/// `X-PAS-Event-Id` header and is also the recommended pattern for the
/// HL7 v2 MLLP peer (use MSH-10 control id as the dedup key).
///
/// A first-failure-wins / all-or-nothing model is the right default for
/// healthcare interop: it's easier to handle a duplicate event than to
/// silently lose one because a single sink went down.
pub struct CompositePublisher {
    children: Vec<Arc<dyn EventPublisher>>,
    /// Which children already took which event, so a retry after a later child failed does not send it to
    /// them again. In memory and bounded (the oldest entries go first): it covers the retries of one process,
    /// which is where the outbox's backoff retries happen; after a restart delivery is at least once, as the
    /// receivers are told (they de-duplicate on `event.id`).
    delivered: std::sync::Mutex<Delivered>,
}

/// The bounded record of (event, child) deliveries.
#[derive(Default)]
struct Delivered {
    set: std::collections::HashSet<(uuid::Uuid, usize)>,
    order: std::collections::VecDeque<(uuid::Uuid, usize)>,
}

/// How many (event, child) deliveries [`CompositePublisher`] remembers.
const DELIVERED_CAP: usize = 50_000;

impl Delivered {
    fn has(&self, key: (uuid::Uuid, usize)) -> bool {
        self.set.contains(&key)
    }
    fn add(&mut self, key: (uuid::Uuid, usize)) {
        if self.set.insert(key) {
            self.order.push_back(key);
            while self.order.len() > DELIVERED_CAP {
                if let Some(old) = self.order.pop_front() {
                    self.set.remove(&old);
                }
            }
        }
    }
    fn forget(&mut self, id: uuid::Uuid, children: usize) {
        for i in 0..children {
            self.set.remove(&(id, i));
        }
        self.order.retain(|(e, _)| *e != id);
    }
}

impl CompositePublisher {
    /// Build a new fan-out publisher. `children` must be non-empty;
    /// callers in `main.rs` are expected to branch on the empty case
    /// and select `InMemoryEventPublisher` directly instead.
    pub fn new(children: Vec<Arc<dyn EventPublisher>>) -> Self {
        debug_assert!(
            !children.is_empty(),
            "CompositePublisher must wrap at least one child"
        );
        Self {
            children,
            delivered: std::sync::Mutex::default(),
        }
    }

    /// Returns the number of downstream publishers. Exposed for logs
    /// and tests.
    pub fn len(&self) -> usize {
        self.children.len()
    }

    /// `true` iff this composite has no downstream publishers — never
    /// `true` in practice because `new` asserts non-empty input. Kept
    /// for completeness and clippy.
    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

#[async_trait]
impl EventPublisher for CompositePublisher {
    async fn publish(&self, event: DomainEvent) -> Result<()> {
        let id = event.id;
        let done = |i: usize| {
            self.delivered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .has((id, i))
        };
        for (i, child) in self.children.iter().enumerate() {
            if done(i) {
                continue;
            }
            child.publish(event.clone()).await?;
            self.delivered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .add((id, i));
        }
        // Every child has it: nothing more to remember for this event.
        self.delivered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .forget(id, self.children.len());
        Ok(())
    }
}

/// Stub Fluvio publisher.
///
/// Always returns `Error::Streaming` — production Fluvio integration is
/// deferred. The type exists so callers can wire the right trait object in
/// configurations that *intend* to use Fluvio.
pub struct FluvioEventPublisher;

impl FluvioEventPublisher {
    /// Construct the stub publisher.
    pub fn new() -> Self {
        Self
    }
}

impl Default for FluvioEventPublisher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventPublisher for FluvioEventPublisher {
    async fn publish(&self, _event: DomainEvent) -> Result<()> {
        Err(crate::Error::Streaming(
            "FluvioEventPublisher not implemented in v0.1".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_in_memory_publish_records_event() {
        let publisher = InMemoryEventPublisher::new();
        let event = DomainEvent::new("PatientCreated", json!({ "id": "abc" }));
        publisher
            .publish(event.clone())
            .await
            .expect("publish should succeed");
        let events = publisher.events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].id, event.id);
        assert_eq!(events[0].event_type, "PatientCreated");
        assert_eq!(events[0].payload, json!({ "id": "abc" }));
    }

    #[tokio::test]
    async fn test_in_memory_publisher_preserves_order() {
        let publisher = InMemoryEventPublisher::new();
        for i in 0..3 {
            publisher
                .publish(DomainEvent::new("EncounterCreated", json!({ "seq": i })))
                .await
                .unwrap();
        }
        let events = publisher.events().await;
        assert_eq!(events.len(), 3);
        for (i, e) in events.iter().enumerate() {
            assert_eq!(e.payload["seq"], i as i64);
        }
    }

    #[tokio::test]
    async fn test_fluvio_publish_returns_err() {
        let publisher = FluvioEventPublisher::new();
        let event = DomainEvent::new("PatientCreated", json!({}));
        let err = publisher
            .publish(event)
            .await
            .expect_err("Fluvio stub must fail");
        match err {
            crate::Error::Streaming(msg) => {
                assert!(msg.contains("not implemented"), "msg: {msg}");
            }
            other => panic!("expected Streaming, got {other:?}"),
        }
    }

    struct FailingPublisher;
    #[async_trait]
    impl EventPublisher for FailingPublisher {
        async fn publish(&self, _event: DomainEvent) -> Result<()> {
            Err(crate::Error::Streaming("boom".into()))
        }
    }

    #[tokio::test]
    async fn test_composite_fans_out_to_all_children() {
        let a = Arc::new(InMemoryEventPublisher::new());
        let b = Arc::new(InMemoryEventPublisher::new());
        let composite = CompositePublisher::new(vec![a.clone(), b.clone()]);
        assert_eq!(composite.len(), 2);
        composite
            .publish(DomainEvent::new("PatientCreated", json!({ "x": 1 })))
            .await
            .expect("ok");
        let a_events = a.events().await;
        let b_events = b.events().await;
        assert_eq!(a_events.len(), 1);
        assert_eq!(b_events.len(), 1);
        assert_eq!(a_events[0].id, b_events[0].id);
    }

    #[tokio::test]
    async fn test_composite_propagates_first_child_failure() {
        let healthy = Arc::new(InMemoryEventPublisher::new());
        let failing = Arc::new(FailingPublisher);
        // Failing child runs first → composite must surface its error.
        let composite = CompositePublisher::new(vec![
            failing.clone(),
            healthy.clone() as Arc<dyn EventPublisher>,
        ]);
        let err = composite
            .publish(DomainEvent::new("PatientCreated", json!({})))
            .await
            .expect_err("must fail");
        match err {
            crate::Error::Streaming(msg) => assert!(msg.contains("boom"), "msg: {msg}"),
            other => panic!("expected Streaming, got {other:?}"),
        }
        // The healthy child was never reached (children run sequentially).
        let healthy_events = healthy.events().await;
        assert!(
            healthy_events.is_empty(),
            "first-failure-wins must short-circuit"
        );
    }

    /// Fails until told to pass, counting its calls.
    struct FlakyPublisher {
        calls: std::sync::atomic::AtomicUsize,
        pass: std::sync::atomic::AtomicBool,
    }
    #[async_trait]
    impl EventPublisher for FlakyPublisher {
        async fn publish(&self, _event: DomainEvent) -> Result<()> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.pass.load(std::sync::atomic::Ordering::SeqCst) {
                Ok(())
            } else {
                Err(crate::Error::Streaming("not yet".into()))
            }
        }
    }

    #[tokio::test]
    async fn test_a_retry_does_not_send_again_to_a_child_that_took_it() {
        let first = Arc::new(InMemoryEventPublisher::new());
        let flaky = Arc::new(FlakyPublisher {
            calls: Default::default(),
            pass: Default::default(),
        });
        let composite = CompositePublisher::new(vec![
            first.clone(),
            flaky.clone() as Arc<dyn EventPublisher>,
        ]);
        let ev = DomainEvent::new("AppointmentBooked", json!({}));
        assert!(composite.publish(ev.clone()).await.is_err());
        assert!(composite.publish(ev.clone()).await.is_err());
        flaky.pass.store(true, std::sync::atomic::Ordering::SeqCst);
        composite.publish(ev.clone()).await.unwrap();
        assert_eq!(
            first.events().await.len(),
            1,
            "the first child got it once, not three times"
        );
        assert_eq!(flaky.calls.load(std::sync::atomic::Ordering::SeqCst), 3);
        // Once every child has it, it is forgotten: a new publish of the same id (a replay) goes to all.
        assert!(composite.delivered.lock().unwrap().set.is_empty());
        composite.publish(ev).await.unwrap();
        assert_eq!(first.events().await.len(), 2);
    }

    #[test]
    fn test_the_record_of_deliveries_is_bounded() {
        let mut d = Delivered::default();
        for _ in 0..(DELIVERED_CAP + 10) {
            d.add((uuid::Uuid::new_v4(), 0));
        }
        assert_eq!(d.set.len(), DELIVERED_CAP);
        assert_eq!(d.order.len(), DELIVERED_CAP);
    }

    #[tokio::test]
    async fn test_composite_with_single_child_is_equivalent() {
        let child = Arc::new(InMemoryEventPublisher::new());
        let composite = CompositePublisher::new(vec![child.clone()]);
        composite
            .publish(DomainEvent::new("PatientCreated", json!({ "x": 7 })))
            .await
            .expect("ok");
        let events = child.events().await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["x"], 7);
    }

    #[test]
    fn test_domain_event_new_assigns_id_and_now() {
        let before = jiff::Timestamp::now();
        let e = DomainEvent::new("X", json!({}));
        let after = jiff::Timestamp::now();
        assert!(e.at >= before && e.at <= after);
        assert_eq!(e.event_type, "X");
        // IDs from different calls differ.
        let e2 = DomainEvent::new("X", json!({}));
        assert_ne!(e.id, e2.id);
    }
}
