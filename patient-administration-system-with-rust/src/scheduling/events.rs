//! Part of [`super`]: events.

use super::*;

impl SchedulingService {
    /// Publish a domain event on this service's publisher (best effort, after a commit).
    pub(crate) async fn publish_event(&self, event: DomainEvent) -> Result<()> {
        self.publisher.publish(event).await
    }

    pub(crate) async fn publish_booked(&self, appt: &Appointment) {
        // A new booking gets its no-show probability, if the score is switched on and approved (CAP-12). It
        // only adds a task for a person; the booking is already made and nothing here can change it.
        if crate::noshow::NoShowService::switched_on()
            && let Some(n) = self.noshow()
        {
            n.assess(appt.id).await.log_warn("no-show assessment");
        }
        self.publisher
            .publish(DomainEvent::new(
                "AppointmentBooked",
                serde_json::json!({ "appointment_id": appt.id }),
            ))
            .await
            .log_warn("in-process event");
    }

    pub(crate) async fn publish_cancelled(&self, appt: &Appointment) {
        // The freed slot is offered to the next patient on standby, when offers are switched on (PILOT-8).
        if let Some(b) = self.backfill()
            && let Err(e) = b.on_cancelled(appt.id).await
        {
            tracing::warn!(target: "pas::backfill", "offer failed: {e}");
        }
        // A cancelled (or moved) appointment no longer needs its ride (CAP-15).
        if let Some(t) = self.transport()
            && let Err(e) = t.on_cancelled(appt.id).await
        {
            tracing::warn!(target: "pas::transport", "transport cancel failed: {e}");
        }
        self.publisher
            .publish(DomainEvent::new(
                "AppointmentCancelled",
                serde_json::json!({ "appointment_id": appt.id }),
            ))
            .await
            .log_warn("in-process event");
    }
}
