import SwiftUI

/// The landing page: server + credentials on ONE page (the web splits this
/// into server-setup → login; same flow, same validation order — URL scheme,
/// reachability, then login). "Use this device's library" is the embedded
/// escape hatch, mirroring the web's standalone-mode footer.
struct ConnectView: View {
    let core: HalogenCore

    @State private var serverUrl = ""
    @State private var username = ""
    @State private var password = ""
    @State private var showPassword = false
    @State private var error: String?
    @State private var connecting = false

    var body: some View {
        ScrollView {
            VStack(spacing: 24) {
                // Add-account arrives here with the previous session intact —
                // give it a way back (an expired session has none).
                if core.addAccountReturnSession != nil {
                    HStack {
                        Button {
                            Task { await core.cancelAddAccount() }
                        } label: {
                            Label("Back", systemImage: "chevron.left")
                        }
                        Spacer()
                    }
                }
                header
                form
                localLibraryFooter
            }
            .padding(24)
            .frame(maxWidth: 480)
        }
        .scrollDismissesKeyboard(.interactively)
        .task {
            // An expired remote session lands here with the server and
            // username already known (web: RootGuard → Login, server
            // pre-known) — only the password is asked again.
            if let hint = core.reauthHint, serverUrl.isEmpty, username.isEmpty {
                serverUrl = hint.serverUrl
                username = hint.username
            }
            await debugAutoconnect()
        }
    }

    /// DEBUG smoke-test hook: `SIMCTL_CHILD_HALOGEN_AUTOCONNECT` drives the
    /// landing page headlessly (`url|user|pass`, or `local` for the embedded
    /// button) — CLI/CI can exercise both auth flows without UI scripting.
    private func debugAutoconnect() async {
        #if DEBUG
            guard let spec = ProcessInfo.processInfo.environment["HALOGEN_AUTOCONNECT"] else {
                return
            }
            if spec == "local" {
                // Detached: the phase change unmounts this view, which would
                // cancel a structured child task mid-login (same reason the
                // button uses Task {}).
                Task { await core.useLocalLibrary() }
                return
            }
            let parts = spec.split(separator: "|").map(String.init)
            guard parts.count == 3 else { return }
            (serverUrl, username, password) = (parts[0], parts[1], parts[2])
            connect()
        #endif
    }

    private var header: some View {
        VStack(spacing: 12) {
            Image(systemName: "waveform.circle.fill")
                .font(.system(size: 56))
                .foregroundStyle(.tint)
            Text("Halogen").font(.largeTitle.bold())
            Text("Connect to your server")
                .font(.subheadline)
                .foregroundStyle(.secondary)
        }
        .padding(.top, 48)
    }

    private var form: some View {
        VStack(spacing: 12) {
            TextField("Server URL (https://…)", text: $serverUrl)
                .textContentType(.URL)
                .keyboardType(.URL)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            TextField("Username", text: $username)
                .textContentType(.username)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
            HStack {
                Group {
                    if showPassword {
                        // Plain field for verifying pasted values — smart
                        // quotes/caps/correction all disabled so what you see
                        // is exactly what's sent.
                        TextField("Password", text: $password)
                            .keyboardType(.asciiCapable)
                            .autocorrectionDisabled()
                            .textInputAutocapitalization(.never)
                    } else {
                        SecureField("Password", text: $password)
                            .textContentType(.password)
                    }
                }
                Button {
                    showPassword.toggle()
                } label: {
                    Image(systemName: showPassword ? "eye.slash" : "eye")
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }

            if let error {
                Text(error)
                    .font(.footnote)
                    .foregroundStyle(.red)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }

            Button(action: connect) {
                if connecting {
                    ProgressView().frame(maxWidth: .infinity)
                } else {
                    Text("Connect").frame(maxWidth: .infinity)
                }
            }
            .buttonStyle(.borderedProminent)
            .disabled(connecting || serverUrl.isEmpty || username.isEmpty || password.isEmpty)
        }
        .textFieldStyle(.roundedBorder)
    }

    private var localLibraryFooter: some View {
        VStack(spacing: 12) {
            HStack {
                VStack { Divider() }
                Text("or").font(.caption).foregroundStyle(.secondary)
                VStack { Divider() }
            }
            Button {
                Task { await core.useLocalLibrary() }
            } label: {
                Label("Use embedded server (this device)", systemImage: "iphone")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.bordered)
            .disabled(connecting)
        }
    }

    private func connect() {
        error = nil
        connecting = true
        Task {
            defer { connecting = false }
            do {
                // Clipboard hygiene: pasted values routinely carry trailing
                // whitespace/newlines the eye can't see. Usernames can't
                // contain them; passwords only lose line breaks + edge spaces.
                try await core.connectRemote(
                    serverUrl: serverUrl.trimmingCharacters(in: .whitespacesAndNewlines),
                    username: username.trimmingCharacters(in: .whitespacesAndNewlines),
                    password: password.trimmingCharacters(in: .newlines)
                )
            } catch {
                self.error = Self.friendly(error)
                DeviceLog.warn("connect: failed — \(error)")
            }
        }
    }

    /// Connect-specific copy layered over the shared mapper: a 401 here means
    /// bad credentials (not an expired session), and an undecodable/empty
    /// answer usually means the URL isn't a Halogen server at all.
    private static func friendly(_ error: Error) -> String {
        switch error {
        case HalogenClient.ClientError.http(401):
            return "Invalid username or password."
        case HalogenClient.ClientError.emptyData, is DecodingError:
            return "The server answered with an unexpected response. Is the URL a Halogen server?"
        case let url as URLError where url.code == .unsupportedURL || url.code == .badURL:
            return "That doesn't look like a valid server URL."
        case let url as URLError
        where [
            .secureConnectionFailed, .serverCertificateUntrusted,
            .serverCertificateHasBadDate, .serverCertificateHasUnknownRoot,
            .serverCertificateNotYetValid,
        ].contains(url.code):
            return "Secure connection failed — check the server's certificate (or use http:// for a local server)."
        default:
            return FriendlyError.message(error)
        }
    }
}
