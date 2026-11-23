package uk.nhs.wales.pas.mobile.core

import java.io.IOException
import java.time.Duration
import java.time.Instant
import java.util.Locale
import java.util.UUID
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class AppointmentsTest {
    private val fixture = checkNotNull(javaClass.getResource("/appointments.json")).readText()
    private val noon = Instant.parse("2026-11-23T12:00:00Z")

    private fun appointment(n: Int, start: Instant) = Appointment(
        UUID.fromString("00000000-0000-4000-8000-%012d".format(n)), DemoAppointmentSource.personId, start,
        start.plus(Duration.ofMinutes(30)), AppointmentStatus.Known.Booked, Modality.Known.InPerson,
    )

    /** FR-227: the envelope decodes, unknown fields are ignored, unknown values are "other", fractional seconds parse. */
    @Test
    fun decodesTheApiEnvelopeAndToleratesNewValues() {
        val items = PasDecoding.appointments(fixture)
        assertEquals(3, items.size)
        assertEquals(AppointmentStatus.Known.Booked, items[0].status)
        assertEquals(Modality.Known.Video, items[0].modality)
        assertEquals(AppointmentStatus.Known.NoShow, items[1].status)
        assertEquals("a missing modality is in person", Modality.Known.InPerson, items[1].modality)
        assertEquals(AppointmentStatus.Other("waiting_for_a_new_status"), items[2].status)
        assertEquals("status.other", items[2].status.messageKey)
        assertEquals("modality.other", items[2].modality.messageKey)
    }

    /** FR-227: upcoming earliest first, past most recent first; one in progress is upcoming. */
    @Test
    fun splitsUpcomingEarliestFirstAndPastMostRecentFirst() {
        val inProgress = appointment(1, noon.minus(Duration.ofMinutes(15)))
        val tomorrow = appointment(2, noon.plus(Duration.ofDays(1)))
        val nextWeek = appointment(3, noon.plus(Duration.ofDays(7)))
        val yesterday = appointment(4, noon.minus(Duration.ofDays(1)))
        val lastMonth = appointment(5, noon.minus(Duration.ofDays(30)))
        val list = AppointmentList.of(listOf(lastMonth, nextWeek, yesterday, inProgress, tomorrow), noon)
        assertEquals(listOf(inProgress, tomorrow, nextWeek), list.upcoming)
        assertEquals(listOf(yesterday, lastMonth), list.past)
        assertFalse(list.isEmpty)
        assertTrue(AppointmentList.of(emptyList(), noon).isEmpty)
    }

    /** FR-227: two appointments at the same time keep one order however they arrive. */
    @Test
    fun equalStartsHaveAStableOrder() {
        val a = appointment(7, noon.plus(Duration.ofHours(1)))
        val b = appointment(8, noon.plus(Duration.ofHours(1)))
        assertEquals(AppointmentList.of(listOf(a, b), noon), AppointmentList.of(listOf(b, a), noon))
    }

    private class Seen {
        var url: String? = null
        var headers: Map<String, String> = emptyMap()
    }

    private fun source(status: Int = 200, body: String = "", fail: Boolean = false, token: String? = "t0ken", seen: Seen = Seen()) =
        HttpAppointmentSource("https://pas.example/", UUID.fromString("6f1a3e0b-0000-4000-8000-000000000042"), { url, headers ->
            seen.url = url
            seen.headers = headers
            if (fail) throw IOException("no route")
            status to body
        }) { token }

    /** The client asks for the one patient's appointments with a bearer credential and decodes the envelope. */
    @Test
    fun httpSourceSendsTheBearerAndDecodes() = runTest {
        val seen = Seen()
        assertEquals(3, source(body = fixture, seen = seen).appointments().size)
        assertEquals("https://pas.example/api/patients/6f1a3e0b-0000-4000-8000-000000000042/appointments", seen.url)
        assertEquals("Bearer t0ken", seen.headers["Authorization"])
    }

    /** FR-228: every failure becomes one of the app's problems; the server's text is never kept. */
    @Test
    fun failuresBecomeTheAppsOwnProblems() = runTest {
        val serverText = """{"title":"SELECT * FROM appointments failed at /srv/pas"}"""
        val cases = listOf(
            source(fail = true) to LoadProblem.Unreachable,
            source(token = null) to LoadProblem.SignedOut,
            source(401, serverText) to LoadProblem.SignedOut,
            source(403, serverText) to LoadProblem.Forbidden,
            source(404, serverText) to LoadProblem.NotFound,
            source(500, serverText) to LoadProblem.Unavailable,
            source(200, serverText) to LoadProblem.Unavailable,
            source(200, "not json") to LoadProblem.Unavailable,
        )
        for ((src, expected) in cases) {
            try {
                src.appointments()
                fail("expected $expected")
            } catch (e: LoadProblemException) {
                assertEquals(expected, e.problem)
                assertFalse(e.message.orEmpty().contains("SELECT"))
            }
        }
    }

    /** FR-228: the model shows a problem, and a person the PAS does not hold has an empty list, not an error. */
    @Test
    fun modelStates() = runTest {
        val failing = AppointmentsModel(source(503), isDemo = false) { noon }
        assertEquals(LoadState.Loading, failing.state.value)
        failing.load()
        assertEquals(LoadState.Failed(LoadProblem.Unavailable), failing.state.value)

        val unknown = AppointmentsModel(source(404), isDemo = false) { noon }
        unknown.load()
        assertEquals(LoadState.Loaded(AppointmentList.of(emptyList(), noon)), unknown.state.value)

        val loaded = AppointmentsModel(source(body = fixture), isDemo = false) { noon }
        loaded.load()
        val list = (loaded.state.value as LoadState.Loaded).list
        assertEquals(2, list.upcoming.size)
        assertEquals(1, list.past.size)
    }

    /** FR-229: the demo has upcoming and past appointments whenever it runs. */
    @Test
    fun demoAlwaysHasUpcomingAndPast() = runTest {
        for (offset in listOf(0L, 200L, -3_000L)) {
            val now = noon.plus(Duration.ofDays(offset))
            val model = AppointmentsModel(DemoAppointmentSource { now }, isDemo = true) { now }
            model.load()
            val list = (model.state.value as LoadState.Loaded).list
            assertTrue(list.upcoming.isNotEmpty())
            assertTrue(list.past.isNotEmpty())
            assertEquals(setOf(DemoAppointmentSource.personId), (list.upcoming + list.past).map { it.personId }.toSet())
        }
    }

    /** Dates are in clinic time and in the locale's words; assert on structure, not digits (AGENTS.md, clock tests). */
    @Test
    fun whenIsInClinicTimeAndTheLocalesWords() {
        // 23:30 UTC on 30 June is 00:30 on 1 July in London (BST): the day must be London's.
        val late = appointment(9, Instant.parse("2026-06-30T23:30:00Z"))
        val english = AppointmentFormat.`when`(late, Locale.forLanguageTag("en-GB"))
        assertTrue(english, english.contains("July"))
        assertFalse(english.contains("June"))
        val welsh = AppointmentFormat.`when`(late, Locale.forLanguageTag("cy-GB"))
        assertNotEquals(english, welsh)
        assertTrue(welsh, welsh.contains("Gorffennaf"))
    }
}
