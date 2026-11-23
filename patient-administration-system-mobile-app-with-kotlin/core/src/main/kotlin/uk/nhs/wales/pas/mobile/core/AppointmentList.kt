package uk.nhs.wales.pas.mobile.core

import java.time.Instant

/**
 * A patient's appointments split for display (FR-227): upcoming, earliest first, then past, most recent first. An appointment
 * is upcoming until it ends, so one in progress is still upcoming. Ties keep a stable order by id.
 */
public data class AppointmentList(val upcoming: List<Appointment>, val past: List<Appointment>) {
    val isEmpty: Boolean get() = upcoming.isEmpty() && past.isEmpty()

    public companion object {
        private val byStart = compareBy<Appointment>({ it.start }, { it.id.toString() })

        public fun of(appointments: List<Appointment>, now: Instant): AppointmentList {
            val (later, earlier) = appointments.partition { it.end.isAfter(now) }
            return AppointmentList(later.sortedWith(byStart), earlier.sortedWith(byStart.reversed()))
        }
    }
}
