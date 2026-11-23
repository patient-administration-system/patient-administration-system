package uk.nhs.wales.pas.mobile.core

import java.time.ZoneId
import java.time.chrono.IsoChronology
import java.time.format.DateTimeFormatter
import java.time.format.DateTimeFormatterBuilder
import java.time.format.FormatStyle
import java.util.Locale

/**
 * Dates as the patient reads them: in clinic time (Europe/London), not the phone's zone, so a patient abroad sees the time on
 * their letter. The words come from the locale.
 */
public object AppointmentFormat {
    public val clinicZone: ZoneId = ZoneId.of("Europe/London")

    public fun `when`(appointment: Appointment, locale: Locale): String {
        val pattern = DateTimeFormatterBuilder.getLocalizedDateTimePattern(
            FormatStyle.FULL, FormatStyle.SHORT, IsoChronology.INSTANCE, locale,
        )
        return DateTimeFormatter.ofPattern(pattern, locale).withZone(clinicZone).format(appointment.start)
    }
}
