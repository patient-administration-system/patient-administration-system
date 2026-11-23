import Foundation

/// A patient's appointments split for display (FR-227): upcoming ones, earliest first, then past ones, most recent first.
/// An appointment is upcoming until it ends, so one in progress is still upcoming. Ties keep a stable order by id.
public struct AppointmentList: Equatable, Sendable {
    public let upcoming: [Appointment]
    public let past: [Appointment]

    public init(_ appointments: [Appointment], now: Date) {
        let (later, earlier) = appointments.reduce(into: ([Appointment](), [Appointment]())) { acc, a in
            if a.end > now { acc.0.append(a) } else { acc.1.append(a) }
        }
        upcoming = later.sorted { ($0.start, $0.id.uuidString) < ($1.start, $1.id.uuidString) }
        past = earlier.sorted { ($0.start, $0.id.uuidString) > ($1.start, $1.id.uuidString) }
    }

    public var isEmpty: Bool { upcoming.isEmpty && past.isEmpty }
}
