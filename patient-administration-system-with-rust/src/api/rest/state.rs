//! Shared application state for the REST API.
//!
//! [`AppState`] is the dependency-injection container that handlers receive
//! through `axum::extract::State`. It owns one `Arc` per service so handlers
//! can clone cheaply on every request.

use std::sync::Arc;

use sea_orm::DatabaseConnection;

use crate::adt::AdtService;
use crate::billing::BillingService;
use crate::communication::{CommunicationService, NoopSmsProvider, SmsProvider};
use crate::resources::ResourcesService;
use crate::scheduling::{SchedulingService, SeriesService};
use crate::streaming::EventPublisher;
use crate::upstream::{PersonClient, WorkerClient};
use crate::waitlist::{RttService, WaitlistService};

/// Application state shared across all HTTP handlers.
///
/// `AppState` is cheap to clone — each field is either a `DatabaseConnection`
/// (already an `Arc` internally) or an `Arc<dyn …>`/`Arc<Service>`.
#[derive(Clone)]
pub struct AppState {
    pub db: DatabaseConnection,
    pub publisher: Arc<dyn EventPublisher>,
    pub adt: Arc<AdtService>,
    pub scheduling: Arc<SchedulingService>,
    pub series: Arc<SeriesService>,
    pub waitlist: Arc<WaitlistService>,
    pub rtt: Arc<RttService>,
    pub resources: Arc<ResourcesService>,
    pub billing: Arc<BillingService>,
    pub communication: Arc<CommunicationService>,
    /// HTTP client to `person-service`. `None` when the service is
    /// not configured (Phase 1 keeps this optional so existing tests don't
    /// need wiring changes; Phase 5 will make it required at startup).
    /// See [`crate::upstream`] and `spec.md` §13 Layer 11.
    pub person_client: Option<Arc<dyn PersonClient>>,
    /// HTTP client to `worker-service`. Same Phase 1 / Phase 5 caveat
    /// as `person_client`.
    pub worker_client: Option<Arc<dyn WorkerClient>>,
}

impl AppState {
    /// Construct a new `AppState`, building one of each service from the
    /// shared database connection and event publisher. The
    /// [`CommunicationService`] starts with [`NoopSmsProvider`] — call
    /// [`Self::with_sms_provider`] before serving requests to enable
    /// real auto-send.
    pub fn new(db: DatabaseConnection, publisher: Arc<dyn EventPublisher>) -> Self {
        Self {
            adt: Arc::new(AdtService::new(db.clone(), publisher.clone())),
            scheduling: Arc::new(SchedulingService::new(db.clone(), publisher.clone())),
            series: Arc::new(SeriesService::new(db.clone(), publisher.clone())),
            waitlist: Arc::new(WaitlistService::new(db.clone(), publisher.clone())),
            rtt: Arc::new(RttService::new(db.clone(), publisher.clone())),
            resources: Arc::new(ResourcesService::new(db.clone(), publisher.clone())),
            billing: Arc::new(BillingService::new(db.clone(), publisher.clone())),
            communication: Arc::new(CommunicationService::new(db.clone(), publisher.clone())),
            person_client: None,
            worker_client: None,
            db,
            publisher,
        }
    }

    /// Attach a [`PersonClient`] for the v0.44.0 identity proxy
    /// surface. Phase 1 builds and attaches it from `main.rs` when
    /// `PERSON_SERVICE_URL` is set; tests can attach a
    /// [`crate::upstream::StubPersonClient`].
    ///
    /// Phase 3b3: the client is also threaded into [`CommunicationService`]
    /// so `generate_letter` can fetch the upstream person snapshot to
    /// bind as the Tera `patient` context variable.
    pub fn with_person_client(mut self, client: Arc<dyn PersonClient>) -> Self {
        // Rebuild communication with the new person client so the renderer
        // can reach upstream. Carrying over the existing SMS provider keeps
        // any earlier `with_sms_provider` configuration intact.
        let sms_provider = self.communication.sms_provider_arc();
        self.communication = Arc::new(
            CommunicationService::new(self.db.clone(), self.publisher.clone())
                .with_sms_provider(sms_provider)
                .with_senders_of(&self.communication)
                .with_person_client(client.clone()),
        );
        self.person_client = Some(client);
        self
    }

    /// Attach a [`WorkerClient`] for the v0.44.0 workforce proxy
    /// surface. Same Phase 1 caveat as
    /// [`Self::with_person_client`].
    pub fn with_worker_client(mut self, client: Arc<dyn WorkerClient>) -> Self {
        self.worker_client = Some(client);
        self
    }

    /// Set the public patient-portal address used for the `portal_url`
    /// template variable.
    pub fn with_portal_base_url(mut self, url: &str) -> Self {
        let mut svc = CommunicationService::new(self.db.clone(), self.publisher.clone())
            .with_sms_provider(self.communication.sms_provider_arc())
            .with_senders_of(&self.communication)
            .with_portal_base_url(url);
        if let Some(c) = self.person_client.clone() {
            svc = svc.with_person_client(c);
        }
        self.communication = Arc::new(svc);
        self
    }

    /// Wire a sender for Email, AppleMessages or WhatsApp.
    pub fn with_sender(
        mut self,
        channel: crate::models::communication::DeliveryChannel,
        sender: Arc<dyn crate::communication::MessageSender>,
    ) -> Self {
        let mut svc = CommunicationService::new(self.db.clone(), self.publisher.clone())
            .with_sms_provider(self.communication.sms_provider_arc())
            .with_senders_of(&self.communication)
            .with_sender(channel, sender);
        if let Some(c) = self.person_client.clone() {
            svc = svc.with_person_client(c);
        }
        self.communication = Arc::new(svc);
        self
    }

    /// Swap in a concrete [`SmsProvider`] for the [`CommunicationService`].
    /// `main.rs` calls this from the config; tests can install
    /// `LogSmsProvider` to exercise the auto-send path without a real
    /// gateway. Has no effect once a request has used `self.communication`
    /// — a fresh service is built around the new provider.
    pub fn with_sms_provider(mut self, sms_provider: Arc<dyn SmsProvider>) -> Self {
        // Rebuild the CommunicationService with the new provider while
        // preserving any previously-attached PersonClient.
        let _ = NoopSmsProvider; // silence unused-import warning when sms is unused
        let mut svc = CommunicationService::new(self.db.clone(), self.publisher.clone())
            .with_sms_provider(sms_provider)
            .with_senders_of(&self.communication);
        if let Some(c) = self.person_client.clone() {
            svc = svc.with_person_client(c);
        }
        self.communication = Arc::new(svc);
        self
    }
}
