package uk.nhs.wales.pas.mobile.core

import java.io.IOException
import java.net.HttpURLConnection
import java.net.URI
import java.time.Duration
import java.time.Instant
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** Why a load failed, in the app's terms. Each has the app's own words (FR-228); no server text is kept. */
public enum class LoadProblem(public val messageKey: String) {
    /** No answer: no connection, a timeout, a refused connection. */
    Unreachable("mobile.error.unreachable"),

    /** 401: the credential was missing, expired or refused. */
    SignedOut("error.401"),

    /** 403: the credential is good but may not read this record. */
    Forbidden("error.403"),

    /** 404: the PAS holds no such person. */
    NotFound("myAppointments.empty"),

    /** Anything else, including a body this build cannot read. */
    Unavailable("error.502"),
    ;

    public companion object {
        public fun forStatus(status: Int): LoadProblem = when (status) {
            401 -> SignedOut
            403 -> Forbidden
            404 -> NotFound
            else -> Unavailable
        }
    }
}

public class LoadProblemException(public val problem: LoadProblem) : Exception(problem.name)

/** Where appointments come from. Throws [LoadProblemException] on failure. */
public fun interface AppointmentSource {
    public suspend fun appointments(): List<Appointment>
}

/** One GET: the status and the body. [HttpUrlConnectionTransport] in the app; a stub in the tests. */
public fun interface Transport {
    /** Throws [IOException] when there is no answer. */
    public suspend fun get(url: String, headers: Map<String, String>): Pair<Int, String>
}

public object HttpUrlConnectionTransport : Transport {
    override suspend fun get(url: String, headers: Map<String, String>): Pair<Int, String> = withContext(Dispatchers.IO) {
        val connection = URI(url).toURL().openConnection() as HttpURLConnection
        try {
            connection.connectTimeout = 15_000
            connection.readTimeout = 30_000
            headers.forEach { (name, value) -> connection.setRequestProperty(name, value) }
            val status = connection.responseCode
            val stream = if (status in 200..299) connection.inputStream else connection.errorStream
            status to (stream?.bufferedReader()?.use { it.readText() } ?: "")
        } finally {
            connection.disconnect()
        }
    }
}

/**
 * Reads `GET /api/patients/{personId}/appointments` with a bearer credential.
 *
 * **Not to be pointed at a real PAS yet** (spec/mobile-apps/index.md, NFR-35): the API trusts its caller to ask only for the
 * patient's own record; the web UI's server enforces that (NFR-3), and nothing does for a phone. The person id must come
 * from a verified token, never from the user, and the API must refuse any other id before this ships.
 */
public class HttpAppointmentSource(
    private val baseUrl: String,
    private val personId: UUID,
    private val transport: Transport = HttpUrlConnectionTransport,
    private val credential: suspend () -> String?,
) : AppointmentSource {
    public val url: String get() = baseUrl.trimEnd('/') + "/api/patients/$personId/appointments"

    override suspend fun appointments(): List<Appointment> {
        val token = credential() ?: throw LoadProblemException(LoadProblem.SignedOut)
        val (status, body) = try {
            transport.get(url, mapOf("Accept" to "application/json", "Authorization" to "Bearer $token"))
        } catch (_: IOException) {
            throw LoadProblemException(LoadProblem.Unreachable)
        }
        if (status !in 200..299) throw LoadProblemException(LoadProblem.forStatus(status))
        return try {
            PasDecoding.appointments(body)
        } catch (_: IllegalArgumentException) {
            throw LoadProblemException(LoadProblem.Unavailable)
        } catch (_: java.time.format.DateTimeParseException) {
            throw LoadProblemException(LoadProblem.Unavailable)
        }
    }
}

/**
 * Example appointments for running the app with no server (FR-229). They are dated from `now`, so the demo always has an
 * upcoming and a past list, and the screen says they are examples.
 */
public class DemoAppointmentSource(private val now: () -> Instant = Instant::now) : AppointmentSource {
    override suspend fun appointments(): List<Appointment> = examples(now())

    public companion object {
        public val personId: UUID = UUID.fromString("00000000-0000-4000-8000-000000000001")

        public fun examples(now: Instant): List<Appointment> = listOf(
            Triple(3L, AppointmentStatus.Known.Booked, Modality.Known.InPerson),
            Triple(10L, AppointmentStatus.Known.Booked, Modality.Known.Video),
            Triple(24L, AppointmentStatus.Known.Proposed, Modality.Known.Phone),
            Triple(-14L, AppointmentStatus.Known.Fulfilled, Modality.Known.InPerson),
            Triple(-40L, AppointmentStatus.Known.Cancelled, Modality.Known.InPerson),
        ).mapIndexed { index, (days, status, modality) ->
            val start = now.plus(Duration.ofDays(days))
            Appointment(
                id = UUID.fromString("00000000-0000-4000-8000-%012d".format(index + 100)),
                personId = personId,
                start = start,
                end = start.plus(Duration.ofMinutes(30)),
                status = status,
                modality = modality,
            )
        }
    }
}
