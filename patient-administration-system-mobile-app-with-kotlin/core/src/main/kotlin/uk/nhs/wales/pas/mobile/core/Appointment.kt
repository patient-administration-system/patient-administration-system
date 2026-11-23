package uk.nhs.wales.pas.mobile.core

import java.time.Instant
import java.util.UUID

/**
 * One appointment as `GET /api/patients/{id}/appointments` returns it (the `Appointment` schema in spec/openapi.json). Only
 * the fields the app shows are kept; the rest are ignored, so a new server field cannot break the app.
 */
public data class Appointment(
    val id: UUID,
    val personId: UUID,
    val start: Instant,
    val end: Instant,
    val status: AppointmentStatus,
    val modality: Modality,
)

/** The lifecycle status. A value this build does not know is [Other], shown as "Other", never a crash. */
public sealed interface AppointmentStatus {
    /** The catalogue key of its label, the same key the web UI uses. */
    public val messageKey: String

    public enum class Known(public val raw: String) : AppointmentStatus {
        Proposed("proposed"), Booked("booked"), Arrived("arrived"), Fulfilled("fulfilled"), Cancelled("cancelled"),
        NoShow("no_show");

        override val messageKey: String get() = "status.$raw"
    }

    public data class Other(val raw: String) : AppointmentStatus {
        override val messageKey: String get() = "status.other"
    }

    public companion object {
        public fun of(raw: String): AppointmentStatus = Known.entries.firstOrNull { it.raw == raw } ?: Other(raw)
    }
}

/** How it is held. An unknown value is [Other]. */
public sealed interface Modality {
    public val messageKey: String

    public enum class Known(public val raw: String) : Modality {
        InPerson("in_person"), Video("video"), Phone("phone");

        override val messageKey: String get() = "modality.$raw"
    }

    public data class Other(val raw: String) : Modality {
        override val messageKey: String get() = "modality.other"
    }

    public companion object {
        public fun of(raw: String): Modality = Known.entries.firstOrNull { it.raw == raw } ?: Other(raw)
    }
}
