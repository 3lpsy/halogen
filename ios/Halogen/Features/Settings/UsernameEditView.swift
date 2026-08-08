import SwiftUI

/// Edit the signed-in user's username (`PUT /users/{id}`, self-or-admin).
/// The stored session updates so the change survives relaunch.
struct UsernameEditView: View {
    let core: HalogenCore

    @Environment(\.dismiss) private var dismiss
    @State private var username: String
    @State private var error: String?
    @State private var saving = false

    init(core: HalogenCore) {
        self.core = core
        _username = State(initialValue: core.account?.username ?? "")
    }

    var body: some View {
        Form {
            TextField("Username", text: $username)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
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
            // Web rules: 3-64 chars; compare the TRIMMED value (a padded
            // copy of the same name used to pass the unchanged guard and PUT
            // an identical name); offline-gated (online-only endpoint).
            .disabled(
                saving || trimmedUsername.count < 3 || trimmedUsername.count > 64
                    || trimmedUsername == core.account?.username || core.isOffline)
            if core.isOffline {
                Text("You're offline — reconnect to rename the account.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
        }
        .navigationTitle("Edit username")
        .navigationBarTitleDisplayMode(.inline)
    }

    private var trimmedUsername: String {
        username.trimmingCharacters(in: .whitespaces)
    }

    private func save() async {
        guard let account = core.account else { return }
        saving = true
        defer { saving = false }
        do {
            try await core.updateUsername(
                userId: account.userId,
                username: username.trimmingCharacters(in: .whitespaces))
            core.renameActiveAccount(to: username.trimmingCharacters(in: .whitespaces))
            ToastCenter.shared.success("Username updated")
            dismiss()
        } catch {
            self.error = FriendlyError.message(error)
        }
    }
}
