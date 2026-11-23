package uk.nhs.wales.pas.mobile

import androidx.annotation.StringRes

/** The string resource for a catalogue key (`Messages.keys`). A key with no resource is a test failure, not a blank screen. */
@StringRes
fun stringFor(key: String): Int = when (key) {
    "myAppointments.title" -> R.string.myAppointments_title
    "myAppointments.empty" -> R.string.myAppointments_empty
    "mobile.upcoming" -> R.string.mobile_upcoming
    "mobile.past" -> R.string.mobile_past
    "mobile.when" -> R.string.mobile_when
    "mobile.how" -> R.string.mobile_how
    "mobile.status" -> R.string.mobile_status
    "mobile.loading" -> R.string.mobile_loading
    "mobile.retry" -> R.string.mobile_retry
    "mobile.demoNotice" -> R.string.mobile_demoNotice
    "mobile.error.title" -> R.string.mobile_error_title
    "mobile.error.unreachable" -> R.string.mobile_error_unreachable
    "error.401" -> R.string.error_401
    "error.403" -> R.string.error_403
    "error.502" -> R.string.error_502
    "status.proposed" -> R.string.status_proposed
    "status.booked" -> R.string.status_booked
    "status.arrived" -> R.string.status_arrived
    "status.fulfilled" -> R.string.status_fulfilled
    "status.cancelled" -> R.string.status_cancelled
    "status.no_show" -> R.string.status_no_show
    "status.other" -> R.string.status_other
    "modality.in_person" -> R.string.modality_in_person
    "modality.video" -> R.string.modality_video
    "modality.phone" -> R.string.modality_phone
    "modality.other" -> R.string.modality_other
    else -> throw IllegalArgumentException("no string resource for $key")
}
