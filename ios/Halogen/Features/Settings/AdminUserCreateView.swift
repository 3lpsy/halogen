import SwiftUI

/// Admin-only "create user with password" form — the counterpart to the web's
/// /admin/users/create page. Online-only, like the rest of user management:
/// the submit goes direct so server errors surface inline, and the server
/// independently enforces the admin gate on POST /admin/users.
struct AdminUserCreateView: View {
    let core: HalogenCore
    /// Called after a successful create so the list can refresh.
    var onCreated: () -> Void = {}

    @Environment(\.dismiss) private var dismiss

    @State private var username = ""
    @State private var password = ""
    @State private var passwordConfirm = ""
    @State private var showPassword = false
    @State private var isAdmin = false
    @State private var submitted = false
    @State private var submitting = false
    @State private var serverError: String?

    var body: some View {
        Form {
            Section {
                TextField("Username", text: $username)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                if let msg = usernameError, submitted {
                    fieldError(msg)
                }
            } footer: {
                Text("3–64 characters.")
            }

            Section {
                HStack {
                    Group {
                        if showPassword {
                            TextField("Password", text: $password)
                        } else {
                            SecureField("Password", text: $password)
                        }
                    }
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    Button {
                        showPassword.toggle()
                    } label: {
                        Image(systemName: showPassword ? "eye.slash" : "eye")
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.borderless)
                    .accessibilityIdentifier("password-eye")
                }
                Group {
                    if showPassword {
                        TextField("Confirm password", text: $passwordConfirm)
                    } else {
                        SecureField("Confirm password", text: $passwordConfirm)
                    }
                }
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                if let msg = passwordError, submitted {
                    fieldError(msg)
                }
            } footer: {
                Text("8–256 characters.")
            }

            Section {
                Toggle("Administrator", isOn: $isAdmin)
            } footer: {
                Text("Administrators can manage users, server settings, and polling.")
            }

            if let serverError {
                Section {
                    Text(serverError)
                        .font(.footnote)
                        .foregroundStyle(.red)
                }
            }

            Section {
                Button {
                    Task { await submit() }
                } label: {
                    if submitting {
                        ProgressView().frame(maxWidth: .infinity)
                    } else {
                        Text("Create user").frame(maxWidth: .infinity)
                    }
                }
                .disabled(submitting || core.isOffline)
            } footer: {
                if core.isOffline {
                    Text("You're offline — reconnect to create users.")
                }
            }
        }
        .navigationTitle("Create user")
        .navigationBarTitleDisplayMode(.inline)
    }

    // Mirrors wire UserStoreData's validators (username 3–64, password
    // 8–256, confirmation must match) so the form and the server agree.
    private var trimmed: String {
        username.trimmingCharacters(in: .whitespaces)
    }

    private var usernameError: String? {
        if trimmed.count < 3 || trimmed.count > 64 {
            return "Username must be between 3 and 64 characters long"
        }
        return nil
    }

    private var passwordError: String? {
        if password.count < 8 || password.count > 256 {
            return "Password must be between 8 and 256 characters long"
        }
        if passwordConfirm != password {
            return "Password confirmation must match the password"
        }
        return nil
    }

    private func fieldError(_ message: String) -> some View {
        Text(message).font(.footnote).foregroundStyle(.red)
    }

    private func submit() async {
        submitted = true
        serverError = nil
        guard usernameError == nil, passwordError == nil else { return }
        submitting = true
        defer { submitting = false }
        do {
            try await core.createUser(
                username: trimmed, password: password, isAdmin: isAdmin)
            ToastCenter.shared.success("User created")
            onCreated()
            dismiss()
        } catch {
            serverError = FriendlyError.message(error)
        }
    }
}
