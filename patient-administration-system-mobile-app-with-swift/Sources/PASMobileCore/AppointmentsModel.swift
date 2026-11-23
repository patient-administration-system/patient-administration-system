import Foundation
import Observation

/// The state of the appointments screen.
public enum LoadState: Equatable, Sendable {
    case loading
    case loaded(AppointmentList)
    case failed(LoadProblem)
}

/// Loads appointments and holds what the screen shows. Pure state, so it is tested without a view.
@MainActor
@Observable
public final class AppointmentsModel {
    public private(set) var state: LoadState = .loading
    public let isDemo: Bool
    private let source: any AppointmentSource
    private let now: @Sendable () -> Date

    public init(source: any AppointmentSource, isDemo: Bool, now: @escaping @Sendable () -> Date = { Date() }) {
        self.source = source
        self.isDemo = isDemo
        self.now = now
    }

    public func load() async {
        state = .loading
        do {
            state = .loaded(AppointmentList(try await source.appointments(), now: now()))
        } catch .notFound {
            // A person the PAS does not hold has, as far as it knows, no appointments.
            state = .loaded(AppointmentList([], now: now()))
        } catch {
            state = .failed(error)
        }
    }
}
