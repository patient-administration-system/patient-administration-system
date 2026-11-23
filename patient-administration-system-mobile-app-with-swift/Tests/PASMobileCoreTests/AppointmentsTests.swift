import Foundation
@testable import PASMobileCore
import Testing

private func fixture() throws -> Data {
    let url = try #require(Bundle.module.url(forResource: "appointments", withExtension: "json", subdirectory: "Fixtures"))
    return try Data(contentsOf: url)
}

private let noon = PASDecoding.parse("2026-11-23T12:00:00Z")!

private func appointment(_ n: Int, start: Date, minutes: Double = 30) -> Appointment {
    Appointment(id: UUID(uuidString: String(format: "00000000-0000-4000-8000-%012d", n))!,
                personId: DemoAppointmentSource.personId, start: start,
                end: start.addingTimeInterval(minutes * 60), status: .booked, modality: .inPerson)
}

/// FR-227: the envelope decodes, unknown fields are ignored, unknown values are "other", fractional seconds parse.
@Test func decodesTheApiEnvelopeAndToleratesNewValues() throws {
    let items = try PASDecoding.appointments(from: try fixture())
    #expect(items.count == 3)
    #expect(items[0].status == .booked)
    #expect(items[0].modality == .video)
    #expect(items[1].status == .noShow)
    #expect(items[1].modality == .inPerson, "a missing modality is in person, the schema's default")
    #expect(items[2].status == .other("waiting_for_a_new_status"))
    #expect(items[2].status.messageKey == "status.other")
    #expect(items[2].modality.messageKey == "modality.other")
}

/// FR-227: upcoming earliest first, past most recent first; one in progress is upcoming.
@Test func splitsUpcomingEarliestFirstAndPastMostRecentFirst() {
    let hour: TimeInterval = 3_600
    let inProgress = appointment(1, start: noon.addingTimeInterval(-0.25 * hour))
    let tomorrow = appointment(2, start: noon.addingTimeInterval(24 * hour))
    let nextWeek = appointment(3, start: noon.addingTimeInterval(7 * 24 * hour))
    let yesterday = appointment(4, start: noon.addingTimeInterval(-24 * hour))
    let lastMonth = appointment(5, start: noon.addingTimeInterval(-30 * 24 * hour))
    let list = AppointmentList([lastMonth, nextWeek, yesterday, inProgress, tomorrow], now: noon)
    #expect(list.upcoming.map(\.id) == [inProgress, tomorrow, nextWeek].map(\.id))
    #expect(list.past.map(\.id) == [yesterday, lastMonth].map(\.id))
    #expect(!list.isEmpty)
    #expect(AppointmentList([], now: noon).isEmpty)
}

/// FR-227: two appointments at the same time keep one order however they arrive.
@Test func equalStartsHaveAStableOrder() {
    let a = appointment(7, start: noon.addingTimeInterval(3_600))
    let b = appointment(8, start: noon.addingTimeInterval(3_600))
    #expect(AppointmentList([b, a], now: noon) == AppointmentList([a, b], now: noon))
}

private struct StubTransport: Transport {
    let status: Int
    let body: Data
    let fail: Bool
    let seen: @Sendable (URLRequest) -> Void
    func send(_ request: URLRequest) async throws -> (Int, Data) {
        seen(request)
        if fail { throw URLError(.notConnectedToInternet) }
        return (status, body)
    }
}

private final class Box: @unchecked Sendable { var request: URLRequest? }

private func source(status: Int = 200, body: Data = Data(), fail: Bool = false, token: String? = "t0ken",
                    box: Box = Box()) -> HTTPAppointmentSource {
    HTTPAppointmentSource(baseURL: URL(string: "https://pas.example/")!,
                          personId: UUID(uuidString: "6F1A3E0B-0000-4000-8000-000000000042")!,
                          transport: StubTransport(status: status, body: body, fail: fail) { box.request = $0 },
                          credential: { token })
}

/// The client asks for the one patient's appointments with a bearer credential and decodes the envelope.
@Test func httpSourceSendsTheBearerAndDecodes() async throws {
    let box = Box()
    let items = try await source(body: try fixture(), box: box).appointments()
    #expect(items.count == 3)
    let request = try #require(box.request)
    #expect(request.url?.absoluteString == "https://pas.example/api/patients/6f1a3e0b-0000-4000-8000-000000000042/appointments")
    #expect(request.value(forHTTPHeaderField: "Authorization") == "Bearer t0ken")
}

/// FR-228: every failure becomes one of the app's problems; the server's text is never kept.
@Test func failuresBecomeTheAppsOwnProblems() async {
    let serverText = Data(#"{"title":"SELECT * FROM appointments failed at /srv/pas"}"#.utf8)
    let cases: [(HTTPAppointmentSource, LoadProblem)] = [
        (source(fail: true), .unreachable),
        (source(token: nil), .signedOut),
        (source(status: 401, body: serverText), .signedOut),
        (source(status: 403, body: serverText), .forbidden),
        (source(status: 404, body: serverText), .notFound),
        (source(status: 500, body: serverText), .unavailable),
        (source(status: 200, body: serverText), .unavailable),
    ]
    for (src, expected) in cases {
        do {
            _ = try await src.appointments()
            Issue.record("expected \(expected)")
        } catch {
            #expect(error == expected)
            for locale in Messages.locales {
                let text = Messages.text(error.messageKey, locale: locale) ?? ""
                #expect(!text.isEmpty)
                #expect(!text.contains("SELECT"))
            }
        }
    }
}

/// FR-228: the model shows a problem, and a person the PAS does not hold has an empty list, not an error.
@MainActor @Test func modelStates() async throws {
    let failing = AppointmentsModel(source: source(status: 503), isDemo: false, now: { noon })
    #expect(failing.state == .loading)
    await failing.load()
    #expect(failing.state == .failed(.unavailable))

    let unknown = AppointmentsModel(source: source(status: 404), isDemo: false, now: { noon })
    await unknown.load()
    #expect(unknown.state == .loaded(AppointmentList([], now: noon)))

    let loaded = AppointmentsModel(source: source(body: try fixture()), isDemo: false, now: { noon })
    await loaded.load()
    guard case .loaded(let list) = loaded.state else { Issue.record("not loaded"); return }
    #expect(list.upcoming.count == 2)
    #expect(list.past.count == 1)
}

/// FR-229: the demo has upcoming and past appointments whenever it runs, and says it is a demo.
@MainActor @Test func demoAlwaysHasUpcomingAndPast() async {
    for offset: TimeInterval in [0, 86_400 * 200, -86_400 * 3_000] {
        let now = noon.addingTimeInterval(offset)
        let model = AppointmentsModel(source: DemoAppointmentSource(now: { now }), isDemo: true, now: { now })
        await model.load()
        guard case .loaded(let list) = model.state else { Issue.record("not loaded"); continue }
        #expect(!list.upcoming.isEmpty)
        #expect(!list.past.isEmpty)
        #expect(Set((list.upcoming + list.past).map(\Appointment.personId)) == [DemoAppointmentSource.personId])
    }
    for locale in Messages.locales {
        #expect(Messages.text("mobile.demoNotice", locale: locale)?.isEmpty == false)
    }
}

/// Dates are in clinic time and in the locale's words; assert on structure, not digits (AGENTS.md, clock tests).
@Test func whenIsInClinicTimeAndTheLocalesWords() {
    // 23:30 UTC on 30 June is 00:30 on 1 July in London (BST): the day must be London's.
    let late = appointment(9, start: PASDecoding.parse("2026-06-30T23:30:00Z")!)
    let english = AppointmentFormat.when(late, locale: Locale(identifier: "en_GB"))
    #expect(english.contains("July"))
    #expect(!english.contains("June"))
    let welsh = AppointmentFormat.when(late, locale: Locale(identifier: "cy_GB"))
    #expect(welsh != english)
    #expect(welsh.contains("Gorffennaf"))
}
