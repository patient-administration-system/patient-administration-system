package uk.nhs.wales.pas.mobile.core

import java.time.Instant
import java.time.OffsetDateTime
import java.util.UUID
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

@Serializable
private data class Envelope<T>(val data: T)

@Serializable
private data class AppointmentJson(
    val id: String,
    @SerialName("person_id") val personId: String,
    @SerialName("start_datetime") val start: String,
    @SerialName("end_datetime") val end: String,
    val status: String,
    // Older servers did not send `modality`; the schema's default is in person.
    val modality: String = "in_person",
)

/**
 * Reads the PAS success envelope, `{"data": ...}`. A failure is an RFC 9457 problem and is never decoded for display: its
 * text is the server's, and the app shows only its own words (FR-228).
 */
public object PasDecoding {
    private val json = Json { ignoreUnknownKeys = true }

    /** RFC 3339 with or without fractional seconds, as chrono writes it. */
    public fun instant(text: String): Instant = OffsetDateTime.parse(text).toInstant()

    /** The appointments inside an envelope; throws [IllegalArgumentException] on a body it cannot read. */
    public fun appointments(body: String): List<Appointment> =
        json.decodeFromString<Envelope<List<AppointmentJson>>>(body).data.map {
            Appointment(
                id = UUID.fromString(it.id),
                personId = UUID.fromString(it.personId),
                start = instant(it.start),
                end = instant(it.end),
                status = AppointmentStatus.of(it.status),
                modality = Modality.of(it.modality),
            )
        }
}
