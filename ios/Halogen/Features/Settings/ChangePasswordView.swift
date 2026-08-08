import SwiftUI

/// Change the signed-in user's password (current + new, confirmed
/// server-side). The web's user-edit page, scoped to the password half —
/// username edits come later with the accounts surface.
struct ChangePasswordView: View {
    let core: HalogenCore

    @Environment(\.dismiss) private var dismiss
    @State private var current = ""
    @State private var new = ""
    @State private var confirm = ""
    @State private var error: String?
    @State private var saving = false

    var body: some View {
        Form {
            Section {
                SecureField("Current password", text: $current)
                SecureField("New password", text: $new)
                SecureField("Confirm new password", text: $confirm)
            } footer: {
                if let error {
                    Text(error).foregroundStyle(.red)
                }
            }
            Button {
                Task { await save() }
            } label: {
                if saving {
                    ProgressView().frame(maxWidth: .infinity)
                } else {
                    Text("Change password").frame(maxWidth: .infinity)
                }
            }
            // Local validation (web local_password_errors: 8-256) + offline
            // gate — a short password round-tripped to a raw server error.
            .disabled(
                saving || current.isEmpty || new.count < 8 || new.count > 256
                    || new != confirm || core.isOffline)
            if !new.isEmpty, new.count < 8 {
                Text("Password must be at least 8 characters.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
            if core.isOffline {
                Text("You're offline — reconnect to change the password.")
                    .font(.footnote).foregroundStyle(.secondary)
            }
        }
        .navigationTitle("Change password")
        .navigationBarTitleDisplayMode(.inline)
    }

    private func save() async {
        saving = true
        defer { saving = false }
        do {
            try await core.changePassword(current: current, new: new)
            // The sheet just closes otherwise — success must be explicit.
            ToastCenter.shared.success("Password changed")
            dismiss()
        } catch {
            self.error = FriendlyError.message(error)
        }
    }
}
