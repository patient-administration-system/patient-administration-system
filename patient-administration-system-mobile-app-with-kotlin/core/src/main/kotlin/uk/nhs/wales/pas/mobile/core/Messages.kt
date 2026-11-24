package uk.nhs.wales.pas.mobile.core

/**
 * The catalogue keys. Keys are shared with the web UI's catalogue where the meaning is the same
 * (`patient-administration-system-ui-with-svelte/src/lib/messages/`); app-only keys start `mobile.`. On Android a key is a
 * string resource named with `.` replaced by `_`.
 */
public object Messages {
    /** The locales with a catalogue. English is the source; the others are first drafts awaiting fluent review. */
    public val locales: List<String> = listOf("en", "cy", "ar", "de", "es", "fr", "hi", "zh")

    /** Every key the app names. */
    public val keys: List<String> =
        AppointmentStatus.Known.entries.map { it.messageKey } + AppointmentStatus.Other("x").messageKey +
            Modality.Known.entries.map { it.messageKey } + Modality.Other("x").messageKey +
            LoadProblem.entries.map { it.messageKey } +
            listOf(
                "myAppointments.title", "mobile.upcoming", "mobile.past", "mobile.when", "mobile.how", "mobile.status",
                "mobile.loading", "mobile.retry", "mobile.demoNotice", "mobile.error.title",
            )

    /** The Android resource name for a key. */
    public fun resourceName(key: String): String = key.replace('.', '_')
}
