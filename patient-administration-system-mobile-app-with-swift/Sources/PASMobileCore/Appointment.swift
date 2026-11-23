import Foundation

/// One appointment as `GET /api/patients/{id}/appointments` returns it (the `Appointment` schema in spec/openapi.json).
/// Only the fields the app shows are decoded; the rest are ignored, so a new server field cannot break the app.
public struct Appointment: Decodable, Equatable, Identifiable, Sendable {
    public let id: UUID
    public let personId: UUID
    public let start: Date
    public let end: Date
    public let status: AppointmentStatus
    public let modality: Modality

    public init(id: UUID, personId: UUID, start: Date, end: Date, status: AppointmentStatus, modality: Modality) {
        self.id = id
        self.personId = personId
        self.start = start
        self.end = end
        self.status = status
        self.modality = modality
    }

    enum CodingKeys: String, CodingKey {
        case id
        case personId = "person_id"
        case start = "start_datetime"
        case end = "end_datetime"
        case status
        case modality
    }

    public init(from decoder: any Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(UUID.self, forKey: .id)
        personId = try c.decode(UUID.self, forKey: .personId)
        start = try c.decode(Date.self, forKey: .start)
        end = try c.decode(Date.self, forKey: .end)
        status = try c.decode(AppointmentStatus.self, forKey: .status)
        // Older servers did not send `modality`; the schema's default is in person.
        modality = try c.decodeIfPresent(Modality.self, forKey: .modality) ?? .inPerson
    }
}

/// The lifecycle status (`AppointmentStatus`). A value this build does not know is `.other`, shown as "Other", never a crash.
public enum AppointmentStatus: Equatable, Sendable, Decodable {
    case proposed, booked, arrived, fulfilled, cancelled, noShow
    case other(String)

    public init(rawValue: String) {
        switch rawValue {
        case "proposed": self = .proposed
        case "booked": self = .booked
        case "arrived": self = .arrived
        case "fulfilled": self = .fulfilled
        case "cancelled": self = .cancelled
        case "no_show": self = .noShow
        default: self = .other(rawValue)
        }
    }

    public init(from decoder: any Decoder) throws {
        self.init(rawValue: try decoder.singleValueContainer().decode(String.self))
    }

    /// The catalogue key of its label, the same key the web UI uses.
    public var messageKey: String {
        switch self {
        case .proposed: "status.proposed"
        case .booked: "status.booked"
        case .arrived: "status.arrived"
        case .fulfilled: "status.fulfilled"
        case .cancelled: "status.cancelled"
        case .noShow: "status.no_show"
        case .other: "status.other"
        }
    }
}

/// How it is held (`Modality`). An unknown value is `.other`.
public enum Modality: Equatable, Sendable, Decodable {
    case inPerson, video, phone
    case other(String)

    public init(rawValue: String) {
        switch rawValue {
        case "in_person": self = .inPerson
        case "video": self = .video
        case "phone": self = .phone
        default: self = .other(rawValue)
        }
    }

    public init(from decoder: any Decoder) throws {
        self.init(rawValue: try decoder.singleValueContainer().decode(String.self))
    }

    public var messageKey: String {
        switch self {
        case .inPerson: "modality.in_person"
        case .video: "modality.video"
        case .phone: "modality.phone"
        case .other: "modality.other"
        }
    }
}
