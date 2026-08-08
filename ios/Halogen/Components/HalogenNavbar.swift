import SwiftUI

extension View {
    /// The app's top navbar: the Halogen title, the reachability dot, and the
    /// user menu (current user / accounts / add account / sign out). Applied
    /// by each tab's ROOT view — pushed views keep their own titles.
    func halogenNavbar(core: HalogenCore) -> some View {
        modifier(HalogenNavbarModifier(core: core))
    }
}

private struct HalogenNavbarModifier: ViewModifier {
    let core: HalogenCore

    @State private var showAccounts = false

    func body(content: Content) -> some View {
        content
            .navigationTitle("Halogen")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    HStack(spacing: 10) {
                        OnlineDot(status: core.connection.status)
                        Menu {
                            Section(core.account?.username ?? "Signed out") {
                                Button {
                                    showAccounts = true
                                } label: {
                                    Label("Accounts", systemImage: "person.2")
                                }
                                Button {
                                    core.beginAddAccount()
                                } label: {
                                    Label("Add account", systemImage: "person.badge.plus")
                                }
                                // Embedded accounts talk to the on-device
                                // server — "going offline" against it is
                                // meaningless and just suspends the outbox
                                // (web hides the toggle there too).
                                if !core.isEmbeddedAccount {
                                    Button {
                                        core.setManualOffline(!core.connection.manualOffline)
                                    } label: {
                                        Label(
                                            core.connection.manualOffline
                                                ? "Go online" : "Go offline",
                                            systemImage: core.connection.manualOffline
                                                ? "wifi" : "wifi.slash")
                                    }
                                }
                                Button(role: .destructive) {
                                    core.signOut()
                                } label: {
                                    Label("Sign out", systemImage: "rectangle.portrait.and.arrow.right")
                                }
                            }
                        } label: {
                            Image(systemName: "person.circle")
                                .font(.title3)
                        }
                    }
                }
            }
            .sheet(isPresented: $showAccounts) {
                NavigationStack {
                    AccountsView(core: core)
                        .toolbar {
                            ToolbarItem(placement: .cancellationAction) {
                                Button("Done") { showAccounts = false }
                            }
                        }
                }
            }
    }
}

/// Green = server reachable, red = not, gray = not probed yet.
struct OnlineDot: View {
    let status: ConnectionMonitor.Status

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 9, height: 9)
            .accessibilityLabel(label)
    }

    private var color: Color {
        switch status {
        case .online: return .green
        case .offline: return .red
        case .unknown: return Color(.systemGray3)
        }
    }

    private var label: String {
        switch status {
        case .online: return "Online"
        case .offline: return "Offline"
        case .unknown: return "Connection unknown"
        }
    }
}
