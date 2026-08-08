import SwiftUI

/// Settings → Accounts: every saved session, switch/remove, add another
/// account (remote login or a new embedded user) — the web's accounts page.
struct AccountsView: View {
    let core: HalogenCore

    @State private var registry = SessionStore.load()
    @State private var confirmRemove: Session?

    var body: some View {
        List {
            Section("Accounts on this device") {
                ForEach(registry.sessions) { session in
                    Button {
                        guard session.id != registry.activeId else { return }
                        Task { await core.switchAccount(session) }
                    } label: {
                        HStack(spacing: 12) {
                            Image(
                                systemName: session.kind == .embedded
                                    ? "iphone" : "server.rack"
                            )
                            .foregroundStyle(.secondary)
                            VStack(alignment: .leading, spacing: 1) {
                                Text(session.username).foregroundStyle(.primary)
                                Text(
                                    session.kind == .embedded
                                        ? "This device" : (session.serverUrl ?? "")
                                )
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                            }
                            Spacer()
                            if session.id == registry.activeId {
                                Image(systemName: "checkmark")
                                    .foregroundStyle(Color.accentColor)
                            }
                        }
                    }
                    .swipeActions(edge: .trailing) {
                        // Destructive parity: every other destructive action
                        // in the app confirms first.
                        Button(role: .destructive) {
                            confirmRemove = session
                        } label: {
                            Label("Remove", systemImage: "trash")
                        }
                    }
                }
            }

            Section {
                Button {
                    core.beginAddAccount()
                } label: {
                    Label("Add account (server login)", systemImage: "plus")
                }
                if case .embedded = core.account?.kind {
                    NavigationLink {
                        AddEmbeddedUserView(core: core)
                    } label: {
                        Label("New user on this device", systemImage: "person.badge.plus")
                    }
                }
                // Admin-only: manage every server user (list / edit /
                // delete) — the web's Accounts "Manage" button.
                if core.isAdmin {
                    NavigationLink {
                        AdminUsersView(core: core)
                    } label: {
                        Label("Manage server users", systemImage: "person.2.badge.gearshape")
                    }
                }
            } footer: {
                Text("Switching accounts swaps the whole library view; each account keeps its own cache and settings. Removing an account signs it out on this device — nothing is deleted on the server.")
            }
        }
        .navigationTitle("Accounts")
        .navigationBarTitleDisplayMode(.inline)
        .onAppear { registry = SessionStore.load() }
        .confirmationDialog(
            "Remove this account?",
            isPresented: Binding(
                get: { confirmRemove != nil },
                set: { if !$0 { confirmRemove = nil } }
            )
        ) {
            if let session = confirmRemove {
                Button("Remove \(session.username)", role: .destructive) {
                    remove(session)
                }
            }
        } message: {
            Text("Signs the account out on this device — nothing is deleted on the server.")
        }
    }

    private func remove(_ session: Session) {
        if session.id == registry.activeId {
            core.signOut()
        } else {
            SessionStore.remove(session.id)
            registry = SessionStore.load()
        }
    }
}

/// Create + switch to a new user on the embedded server (username only — the
/// app generates and stores the password), the web's add-embedded-user page.
struct AddEmbeddedUserView: View {
    let core: HalogenCore

    @State private var username = ""
    @State private var error: String?
    @State private var saving = false

    var body: some View {
        Form {
            Section {
                TextField("Username", text: $username)
                    .autocorrectionDisabled()
                    .textInputAutocapitalization(.never)
                // Visible but locked, like the web: every embedded user is an
                // admin of the on-device server (no lesser role to manage).
                Toggle("Administrator", isOn: .constant(true))
                    .disabled(true)
            } footer: {
                Text("Users on the embedded server are always administrators. No password needed — the app manages sign-in.")
            }
            if let error {
                Text(error).font(.footnote).foregroundStyle(.red)
            }
            Button {
                Task { await create() }
            } label: {
                if saving {
                    ProgressView().frame(maxWidth: .infinity)
                } else {
                    Text("Create and switch").frame(maxWidth: .infinity)
                }
            }
            .disabled(saving || username.trimmingCharacters(in: .whitespaces).isEmpty)
        }
        .navigationTitle("New device user")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func create() async {
        saving = true
        defer { saving = false }
        do {
            try await core.addEmbeddedUser(username: username)
        } catch {
            self.error = FriendlyError.message(error)
        }
    }
}
