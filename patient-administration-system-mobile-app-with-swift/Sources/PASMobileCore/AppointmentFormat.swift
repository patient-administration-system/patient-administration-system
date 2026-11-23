import Foundation

/// Dates as the patient reads them: in clinic time (Europe/London), not the phone's zone, so a patient abroad sees the time
/// on their letter. The words come from the locale.
public enum AppointmentFormat {
    public static let clinicTimeZone = TimeZone(identifier: "Europe/London")!

    /// Day, date and start time, e.g. "Tuesday 24 November 2026 at 09:30" in English.
    public static func when(_ appointment: Appointment, locale: Locale) -> String {
        var style = Date.FormatStyle(date: .complete, time: .shortened, locale: locale, timeZone: clinicTimeZone)
        style.calendar = Calendar(identifier: .gregorian)
        return appointment.start.formatted(style)
    }
}
