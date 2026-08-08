import SwiftUI

/// Admin-only user management — the web's AdminUsers page: every server user
/// (`GET /users`), edit one, delete one. Online-only (no outbox, no cache);
/// the server is the real authority for the admin gate and the self-delete
/// refusal. Entry point: Settings → Accounts → Manage server users.
struct AdminUsersView: View {
    let core: HalogenCore

    @State private var users: [UserData] = []
    @State private var loadError: String?
    @State private var pendingDelete: UserData?
    @State private var deleting = false

    var body: some View {
        Group {
            if let loadError {
                ContentUnavailableView {
                    Label("Couldn't load users", systemImage: "wifi.exclamationmark")
                } description: {
                    Text(loadError).font(.footnote.monospaced())
                }
            } else {
                List {
                    if core.isOffline {
                        Section {
                            Text("You're offline — reconnect to manage users.")
                                .font(.footnote)
                                .foregroundStyle(.orange)
                        }
                    }
                    Section {
                        ForEach(users, id: \.id) { user in
                            NavigationLink {
                                AdminUserEditView(core: core, user: user) {
                                    Task { await load() }
                                }
                            } label: {
                                row(user)
                            }
                            .disabled(core.isOffline)
                            .swipeActions(edge: .trailing) {
                                // Can't delete your own account — another
                                // admin must do it (web parity).
                                if !isSelf(user) {
                                    Button(role: .destructive) {
                                        pendingDelete = user
                                    } label: {
                                        Label("Delete", systemImage: "trash")
                                    }
                                    .disabled(core.isOffline || deleting)
                                }
                            }
                        }
                    } footer: {
                        if !users.isEmpty {
                            Text("Swipe to delete a user. You can't delete your own account.")
                        }
                    }
                }
            }
        }
        .navigationTitle("Users")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                NavigationLink {
                    AdminUserCreateView(core: core) {
                        Task { await load() }
                    }
                } label: {
                    Label("Create user", systemImage: "plus")
                }
                .disabled(core.isOffline)
            }
        }
        .task { await load() }
        .refreshable { await load() }
        .confirmationDialog(
            "Delete user?",
            isPresented: Binding(
                get: { pendingDelete != nil },
                set: { if !$0 { pendingDelete = nil } }
            ),
            titleVisibility: .visible
        ) {
            Button("Delete", role: .destructive) {
                if let user = pendingDelete {
                    Task { await delete(user) }
                }
            }
        } message: {
            Text(
                "This permanently deletes the account \"\(pendingDelete?.username ?? "")\". This can't be undone."
            )
        }
    }

    private func row(_ user: UserData) -> some View {
        HStack(spacing: 8) {
            Text(user.username.isEmpty ? "User \(user.id)" : user.username)
            if user.is_admin {
                Text("Admin")
                    .font(.caption2.weight(.semibold))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Color.accentColor.opacity(0.15)))
                    .foregroundStyle(Color.accentColor)
            }
            if isSelf(user) {
                Text("You")
                    .font(.caption2.weight(.semibold))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Capsule().fill(Color.secondary.opacity(0.15)))
                    .foregroundStyle(.secondary)
            }
        }
    }

    private func isSelf(_ user: UserData) -> Bool {
        user.id == core.account?.userId
    }

    private func load() async {
        do {
            users = try await core.listUsers()
            loadError = nil
        } catch {
            if users.isEmpty { loadError = FriendlyError.message(error) }
        }
    }

    private func delete(_ user: UserData) async {
        deleting = true
        defer {
            deleting = false
            pendingDelete = nil
        }
        do {
            try await core.deleteUser(id: user.id)
            ToastCenter.shared.success("User deleted")
            await load()
        } catch {
            ToastCenter.shared.error("Delete failed: \(error)")
        }
    }
}

/// Admin edit page for another user — username + the Is Admin toggle (`PUT
/// /users/{id}`), the web's AdminUserEdit / AccountDetailsForm with
/// `admin: Some(current)`. There's no password field (admins can't change
/// another user's password).
struct AdminUserEditView: View {
    let core: HalogenCore
    let user: UserData
    var onSaved: () -> Void = {}

    @Environment(\.dismiss) private var dismiss
    @State private var username: String
    @State private var isAdmin: Bool
    @State private var error: String?
    @State private var saving = false

    init(core: HalogenCore, user: UserData, onSaved: @escaping () -> Void = {}) {
        self.core = core
        self.user = user
        self.onSaved = onSaved
        _username = State(initialValue: user.username)
        _isAdmin = State(initialValue: user.is_admin)
    }

    private var trimmed: String {
        username.trimmingCharacters(in: .whitespaces)
    }

    private var usernameChanged: Bool {
        trimmed != user.username.trimmingCharacters(in: .whitespaces)
    }

    private var unchanged: Bool {
        !usernameChanged && isAdmin == user.is_admin
    }

    var body: some View {
        Form {
            TextField("Username", text: $username)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            Toggle("Administrator", isOn: $isAdmin)
            if let error {
                Text(error).font(.footnote).foregroundStyle(.red)
            }
            Button {
                Task { await save() }
            } label: {
                if saving {
                    ProgressView().frame(maxWidth: .infinity)
                } else {
                    Text("Save").frame(maxWidth: .infinity)
                }
            }
            .disabled(
                saving || unchanged || core.isOffline
                    || (usernameChanged && trimmed.count < 3))
        }
        .navigationTitle("Edit user")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func save() async {
        saving = true
        defer { saving = false }
        do {
            // Only send the username when it actually changed (web parity:
            // an admin-only flag toggle must not re-validate a legacy name).
            try await core.updateUser(
                userId: user.id,
                username: usernameChanged ? trimmed : nil,
                isAdmin: isAdmin)
            if user.id == core.account?.userId, usernameChanged {
                core.renameActiveAccount(to: trimmed)
            }
            ToastCenter.shared.success("User updated")
            onSaved()
            dismiss()
        } catch {
            self.error = FriendlyError.message(error)
        }
    }
}
