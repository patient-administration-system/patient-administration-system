import Foundation

/// Why a load failed, in the app's terms. Each case has the app's own words (FR-228); no server text is kept.
public enum LoadProblem: Error, Equatable, Sendable {
    /// No answer: no connection, a timeout, a refused connection.
    case unreachable
    /// 401: the credential was missing, expired or refused.
    case signedOut
    /// 403: the credential is good but may not read this record.
    case forbidden
    /// 404: the PAS holds no such person.
    case notFound
    /// Anything else, including a body this build cannot read.
    case unavailable

    public var messageKey: String {
        switch self {
        case .unreachable: "mobile.error.unreachable"
        case .signedOut: "error.401"
        case .forbidden: "error.403"
        case .notFound: "myAppointments.empty"
        case .unavailable: "error.502"
        }
    }

    /// The problem for an HTTP status. A 2xx is not a problem and is not passed here.
    public static func forStatus(_ status: Int) -> LoadProblem {
        switch status {
        case 401: .signedOut
        case 403: .forbidden
        case 404: .notFound
        default: .unavailable
        }
    }
}

/// Where appointments come from.
public protocol AppointmentSource: Sendable {
    func appointments() async throws(LoadProblem) -> [Appointment]
}

/// Sends one request and returns the status and body. URLSession in the app; a stub in the tests.
public protocol Transport: Sendable {
    func send(_ request: URLRequest) async throws -> (Int, Data)
}

public struct URLSessionTransport: Transport {
    private let session: URLSession
    public init(session: URLSession = .shared) { self.session = session }
    public func send(_ request: URLRequest) async throws -> (Int, Data) {
        let (data, response) = try await session.data(for: request)
        return ((response as? HTTPURLResponse)?.statusCode ?? 0, data)
    }
}

/// Reads `GET /api/patients/{personId}/appointments` with a bearer credential.
///
/// **Not to be pointed at a real PAS yet** (spec/mobile-apps/index.md, NFR-35): the API trusts its caller to ask only for the
/// patient's own record; the web UI's server enforces that (NFR-3), and nothing does for a phone. The person id here must
/// come from a verified token, never from the user, and the API must refuse any other id before this ships.
public struct HTTPAppointmentSource: AppointmentSource {
    public let baseURL: URL
    public let personId: UUID
    private let credential: @Sendable () async -> String?
    private let transport: any Transport

    public init(baseURL: URL, personId: UUID, transport: any Transport = URLSessionTransport(),
                credential: @escaping @Sendable () async -> String?) {
        self.baseURL = baseURL
        self.personId = personId
        self.transport = transport
        self.credential = credential
    }

    public var request: URLRequest {
        let url = baseURL.appending(path: "api/patients/\(personId.uuidString.lowercased())/appointments")
        var request = URLRequest(url: url)
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        return request
    }

    public func appointments() async throws(LoadProblem) -> [Appointment] {
        guard let token = await credential() else { throw .signedOut }
        var request = self.request
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        let status: Int
        let body: Data
        do { (status, body) = try await transport.send(request) } catch { throw .unreachable }
        guard (200..<300).contains(status) else { throw LoadProblem.forStatus(status) }
        do { return try PASDecoding.appointments(from: body) } catch { throw .unavailable }
    }
}

/// Example appointments for running the app with no server (FR-229). They are dated from `now`, so the demo always has an
/// upcoming and a past list, and the screen says they are examples.
public struct DemoAppointmentSource: AppointmentSource {
    public static let personId = UUID(uuidString: "00000000-0000-4000-8000-000000000001")!
    private let now: @Sendable () -> Date

    public init(now: @escaping @Sendable () -> Date = { Date() }) { self.now = now }

    public func appointments() async throws(LoadProblem) -> [Appointment] {
        Self.examples(now: now())
    }

    public static func examples(now: Date) -> [Appointment] {
        let day: TimeInterval = 86_400
        let items: [(TimeInterval, AppointmentStatus, Modality)] = [
            (3 * day, .booked, .inPerson),
            (10 * day, .booked, .video),
            (24 * day, .proposed, .phone),
            (-14 * day, .fulfilled, .inPerson),
            (-40 * day, .cancelled, .inPerson),
        ]
        return items.enumerated().map { index, item in
            let start = now.addingTimeInterval(item.0)
            return Appointment(
                id: UUID(uuidString: String(format: "00000000-0000-4000-8000-%012d", index + 100))!,
                personId: personId,
                start: start,
                end: start.addingTimeInterval(30 * 60),
                status: item.1,
                modality: item.2
            )
        }
    }
}
