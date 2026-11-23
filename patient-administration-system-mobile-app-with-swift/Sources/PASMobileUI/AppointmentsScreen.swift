import LilyThemes
import LilyUI
import PASMobileCore
import SwiftUI

/// The patient's appointments (FR-227): upcoming first, then past, each as a Lily card with a summary list.
public struct AppointmentsScreen: View {
    @State private var model: AppointmentsModel
    @Environment(\.colorScheme) private var colorScheme

    public init(model: AppointmentsModel) {
        _model = State(initialValue: model)
    }

    public var body: some View {
        NavigationStack {
            content
                .navigationTitle(Messages.text("myAppointments.title"))
        }
        .lilyTheme(colorScheme == .dark ? .dark : .light)
        .task { await model.load() }
    }

    @ViewBuilder private var content: some View {
        switch model.state {
        case .loading:
            ProgressView(Messages.text("mobile.loading"))
        case .failed(let problem):
            VStack(alignment: .leading, spacing: 16) {
                ErrorSummary(title: Messages.text("mobile.error.title"),
                             problems: [.init(Messages.text(problem.messageKey)) {}])
                Button(Messages.text("mobile.retry")) { Task { await model.load() } }
                    .frame(minHeight: 44)
            }
            .padding()
        case .loaded(let list):
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    if model.isDemo {
                        InsetText { Text(Messages.text("mobile.demoNotice")) }
                    }
                    if list.isEmpty {
                        Text(Messages.text("myAppointments.empty"))
                    }
                    section("mobile.upcoming", list.upcoming)
                    section("mobile.past", list.past)
                }
                .padding()
            }
            .refreshable { await model.load() }
        }
    }

    /// A heading in the text colour (the system's grey section header does not reach 4.5:1) and the section's cards.
    @ViewBuilder private func section(_ key: String, _ items: [Appointment]) -> some View {
        if !items.isEmpty {
            Text(Messages.text(key))
                .font(.title2.bold())
                .accessibilityAddTraits(.isHeader)
            ForEach(items) { row($0) }
        }
    }

    private func row(_ appointment: Appointment) -> some View {
        let when = AppointmentFormat.when(appointment, locale: Messages.dateLocale)
        return Card(label: when) {
            SummaryList(label: when) {
                SummaryListItem(term: Messages.text("mobile.when")) { Text(when) }
                SummaryListItem(term: Messages.text("mobile.how")) { Text(Messages.text(appointment.modality.messageKey)) }
                SummaryListItem(term: Messages.text("mobile.status")) {
                    let status = Messages.text(appointment.status.messageKey)
                    Tag(status, label: Messages.text("mobile.status"))
                }
            }
        }
    }
}
